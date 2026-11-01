// ============================================================================
// CREATE :: KINETIC CORE — hand-written Rust port (no transpiler).
// Rotation network over Horizon block entities: sources, shafts, cogs
// (direction flip), gearboxes (axis pass-through), consumers (stress).
// State lives in each block entity's NBT (persists across saves) and in a
// module network map rebuilt on block-entity attach + block changes.
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KinKind {
    Shaft,
    Cog,
    LargeCog,
    Gearbox,
    Source,
    Consumer,
}

/// Create's kinetic blocks this build implements. Unknown kinetic blocks are
/// treated as shafts once their block entity exists.
pub fn kinetic_kind(block: &str) -> Option<KinKind> {
    Some(match block {
        "create:shaft"
        | "create:andesite_encased_shaft"
        | "create:brass_encased_shaft" => KinKind::Shaft,
        "create:cogwheel" | "create:andesite_encased_cogwheel"
        | "create:brass_encased_cogwheel" => KinKind::Cog,
        "create:large_cogwheel" | "create:andesite_encased_large_cogwheel"
        | "create:brass_encased_large_cogwheel" => KinKind::LargeCog,
        "create:gearbox" => KinKind::Gearbox,
        "create:water_wheel" | "create:steam_engine" | "create:flywheel" => KinKind::Source,
        "create:millstone" | "create:crusher" | "create:mechanical_press"
        | "create:mechanical_drill" | "create:mechanical_saw" | "create:deployer"
        | "create:mechanical_mixer" => KinKind::Consumer,
        _ => return None,
    })
}

/// Source RPM at the block (Create: water wheel 16, steam engine 32).
pub fn source_rpm(block: &str) -> f32 {
    match block {
        "create:water_wheel" => 16.0,
        "create:steam_engine" => 32.0,
        "create:flywheel" => 16.0,
        _ => 0.0,
    }
}

/// Stress units a consumer demands (Create-ish values, rounded).
pub fn stress_demand(block: &str) -> u32 {
    match block {
        "create:millstone" => 4,
        "create:crusher" => 8,
        "create:mechanical_press" => 8,
        "create:mechanical_drill" => 8,
        "create:mechanical_saw" => 8,
        "create:deployer" => 4,
        "create:mechanical_mixer" => 8,
        _ => 0,
    }
}

/// Stress capacity a source provides.
pub fn stress_capacity(block: &str) -> u32 {
    match block {
        "create:water_wheel" => 128,
        "create:steam_engine" => 256,
        "create:flywheel" => 512,
        _ => 0,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct KinNode {
    /// The block's own registered name (`create:cogwheel`).
    pub block: &'static str,
    pub kind: KinKind,
}

/// The network: position -> node. Game-thread only (single-threaded ticks),
/// so a thread-local RefCell keeps it lock-free.
thread_local! {
    pub static NET: RefCell<Network> = RefCell::new(Network::default());
}

#[derive(Default)]
pub struct Network {
    pub nodes: BTreeMap<[i32; 3], KinNode>,
    /// Recomputed propagation: pos -> (rpm, direction sign).
    pub speeds: BTreeMap<[i32; 3], (f32, i32)>,
    pub dirty: bool,
    pub stress: (u32, u32),
}

pub fn net_attach(pos: [i32; 3], block: &'static str) {
    let Some(kind) = kinetic_kind(block) else { return };
    NET.with(|n| {
        let mut net = n.borrow_mut();
        net.nodes.insert(pos, KinNode { block, kind });
        net.dirty = true;
    });
}

pub fn net_detach(pos: [i32; 3]) {
    NET.with(|n| {
        let mut net = n.borrow_mut();
        if net.nodes.remove(&pos).is_some() {
            net.dirty = true;
        }
    });
}

/// Propagate: BFS from every source over adjacency (6-neighbors; a cog also
/// reaches the neighbor along its meshing side, flipping direction; a large
/// cog halves RPM, a small cog driven by a large doubles it). Overstressed
/// networks stop (speed 0) — as Create's overstressed does.
pub fn net_recompute() {
    NET.with(|n| {
        let mut net = n.borrow_mut();
        if !net.dirty {
            return;
        }
        net.speeds.clear();
        let mut demand = 0u32;
        let mut capacity = 0u32;
        let sources: Vec<[i32; 3]> = net
            .nodes
            .iter()
            .filter(|(_, node)| node.kind == KinKind::Source)
            .map(|(p, _)| *p)
            .collect();
        for src in sources {
            let rpm = source_rpm(net.nodes[&src].block);
            capacity += stress_capacity(net.nodes[&src].block);
            let mut seen: BTreeMap<[i32; 3], (f32, i32)> = BTreeMap::new();
            let mut queue = std::collections::VecDeque::new();
            queue.push_back((src, rpm, 1i32));
            seen.insert(src, (rpm, 1));
            while let Some((pos, speed, dir)) = queue.pop_front() {
                if let Some(node) = net.nodes.get(&pos) {
                    if node.kind == KinKind::Consumer {
                        demand += stress_demand(node.block);
                    }
                }
                for d in [
                    [1, 0, 0],
                    [-1, 0, 0],
                    [0, 1, 0],
                    [0, -1, 0],
                    [0, 0, 1],
                    [0, 0, -1],
                ] {
                    let next = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                    let Some(node) = net.nodes.get(&next) else { continue };
                    if seen.contains_key(&next) {
                        continue;
                    }
                    let (nspeed, ndir) = match node.kind {
                        KinKind::Shaft | KinKind::Gearbox | KinKind::Source => (speed, dir),
                        KinKind::Cog => (speed, -dir),
                        KinKind::LargeCog => (speed / 2.0, -dir),
                        KinKind::Consumer => (speed, dir),
                    };
                    seen.insert(next, (nspeed, ndir));
                    queue.push_back((next, nspeed, ndir));
                }
            }
            for (p, sd) in seen {
                net.speeds.insert(p, sd);
            }
        }
        if demand > capacity && capacity > 0 {
            for sd in net.speeds.values_mut() {
                sd.0 = 0.0;
            }
        }
        net.stress = (demand, capacity);
        net.dirty = false;
    });
}

pub fn net_speed(pos: [i32; 3]) -> (f32, i32, u32, u32) {
    NET.with(|n| {
        let net = n.borrow();
        let (rpm, dir) = net.speeds.get(&pos).copied().unwrap_or((0.0, 0));
        (rpm, dir, net.stress.0, net.stress.1)
    })
}


// ============================================================================
// v0.3: consumers process the world; tooltips from Create's lang.
// ============================================================================

/// Create's crushing chain, block to block (what the player SEES happen).
pub fn crush_step(name: &str) -> Option<&'static str> {
    Some(match name {
        "minecraft:stone" => "minecraft:cobblestone",
        "minecraft:cobblestone" => "minecraft:gravel",
        "minecraft:gravel" => "minecraft:sand",
        "minecraft:sandstone" => "minecraft:sand",
        "minecraft:iron_ore" => "minecraft:raw_iron_block",
        "minecraft:deepslate_iron_ore" => "minecraft:raw_iron_block",
        "minecraft:gold_ore" => "minecraft:raw_gold_block",
        "minecraft:deepslate_gold_ore" => "minecraft:raw_gold_block",
        "minecraft:copper_ore" => "minecraft:raw_copper_block",
        "minecraft:deepslate_copper_ore" => "minecraft:raw_copper_block",
        _ => return None,
    })
}

/// Create's tooltip summaries for the kinetic blocks (en_us.json).
pub fn tooltip_summary(item: &str) -> Option<&'static str> {
    Some(match item {
        "create:shaft" => "Relays rotation from one kinetic block to another.",
        "create:cogwheel" => "Relays rotation and can change the axis of it, or mesh with other cogwheels.",
        "create:large_cogwheel" => "A larger and more durable cogwheel. Changes the speed of a rotation network by 1:2.",
        "create:gearbox" => "Relays rotation between its axis, and to the sides independently.",
        "create:water_wheel" => "Generates rotational force from adjacent flowing water.",
        "create:steam_engine" => "A source of rotational power, fueled by burning any fluid fuel and water.",
        "create:millstone" => "Grinds what is placed beneath it while the network spins.",
        "create:mechanical_drill" => "Breaks or crushes the block it faces while the network spins.",
        "create:crusher" => "Crushes the block beneath it while the network spins.",
        _ => return None,
    })
}

/// The processing tick of a consumer: every 60 spinning ticks convert the
/// block BELOW through the crushing chain (world read -> set_block).
pub fn consumer_tick(ctx: &mut BridgeCtx, pos: [i32; 3], counter: &mut i32) {
    let (rpm, _dir, demand, capacity) = net_speed(pos);
    if rpm <= 0.0 || demand > capacity {
        return;
    }
    *counter += 1;
    if *counter < 60 {
        return;
    }
    *counter = 0;
    let below = [pos[0], pos[1] - 1, pos[2]];
    let name = ctx.block_name_at(below);
    if let Some(next) = crush_step(&name) {
        ctx.set_block_by_name(below, next);
    }
}


// ============================================================================
// v0.4: the Ponder guide — a right-clickable block whose native screen shows
// Create's Ponder pages (header + texts from the mod's lang data).
// ============================================================================

pub const GUIDE_BLOCK: &str = "create:ponder";

/// Which block's screen the guide is. Horizon asks per block entity.
pub fn guide_screen_attach(block: &str, screen: &mut Option<crate::game::native_hooks::NativeScreen>) {
    if block == GUIDE_BLOCK {
        *screen = Some(crate::game::native_hooks::NativeScreen {
            width: 176.0,
            height: 166.0,
            slots: Vec::new(),
            inventory_y: 166.0,
            inventory_x: 8.0,
            own_labels: true,
        });
    }
}

/// One subject per page: header + wrapped text lines, GUI-pixel layout.
pub fn guide_page(page: usize) -> (&'static str, Vec<&'static str>) {
    let idx = page % GUIDE.len();
    let (subject, header, texts) = GUIDE[idx];
    let mut lines: Vec<&'static str> = vec![header];
    for t in texts.iter() {
        // hard wrap at ~40 chars for the 176-wide screen
        let mut line = String::new();
        for w in t.split(' ') {
            if line.chars().count() + w.chars().count() + 1 > 40 {
                lines.push(Box::leak(line.clone().into_boxed_str()) as &'static str);
                line.clear();
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(w);
        }
        if !line.is_empty() {
            lines.push(Box::leak(line.into_boxed_str()) as &'static str);
        }
        lines.push("");
    }
    lines.push(Box::leak(format!("{}. {}", idx + 1, subject).into_boxed_str()) as &'static str);
    (subject, lines)
}

pub fn guide_page_of(data: &[i32]) -> usize {
    let raw = data.first().copied().unwrap_or(0);
    (raw.max(0) as usize) % GUIDE.len()
}
