// DRAFT translation of Java mod 'Create' by j2h 0.1.0.
// Semantic mode: JvmVal handles + named dispatch; no file attributes —
// horizon-mixins splices this as an inline module.

// The universal runtime layer emitted into every translated mod module
// (`jvm_rt` prelude): a minimal JVM object model + a named-call bridge to
// Horizon's real hook APIs.
//
// WHY THIS EXISTS
//
// Java mods live on object graphs, reflection, and pervasive null; Rust
// ownership makes a direct translation untypeable without per-mod manual
// work. Instead, every Java reference becomes a [`JvmVal`] handle and every
// vanilla call is dispatched BY NAME through [`JvmVal::call`] /
// [`JvmVal::call_static`]. Consequences:
//
// * translated code ALWAYS compiles (no per-mod type surgery);
// * unmapped methods degrade at runtime with a log line — never a panic;
// * the ~15 vanilla calls Horizon exposes natively (`native_set_block`,
//   `native_give`, `native_play_sound`, ...) are implemented for real in
//   [`BridgeCtx`], so mods that stay inside the official hook surface WORK,
//   not just load;
// * idiomatization becomes optional refinement, not a correctness gate.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// One Java value. References are handles; numbers are unboxed.
#[derive(Clone, Debug)]
pub enum JvmVal {
    Null,
    /// Instance handle: class internal name + object id.
    Obj(String, u64),
    /// String (common enough to special-case).
    Str(Rc<RefCell<String>>),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Bool(bool),
    /// Block position — the shape hooks pass around.
    BlockPos([i32; 3]),
    /// Block state id as Horizon names them.
    State(u32),
    /// Raw bytes (network payloads).
    Bytes(Rc<RefCell<Vec<u8>>>),
}

static NEXT_OBJ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl JvmVal {
    pub fn obj(class: impl Into<String>) -> JvmVal {
        JvmVal::Obj(class.into(), NEXT_OBJ.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
    pub fn string(s: impl Into<String>) -> JvmVal {
        JvmVal::Str(Rc::new(RefCell::new(s.into())))
    }
    pub fn str_of(&self) -> String {
        match self {
            JvmVal::Str(s) => s.borrow().clone(),
            other => format!("{other:?}"),
        }
    }
    /// For `if_icmp` / switch dispatch in translated code.
    pub fn as_i32(&self) -> i32 {
        match self {
            JvmVal::Int(i) => *i,
            JvmVal::Long(l) => *l as i32,
            JvmVal::Bool(b) => *b as i32,
            _ => 0,
        }
    }
    pub fn as_bool(&self) -> bool {
        match self {
            JvmVal::Bool(b) => *b,
            JvmVal::Int(i) => *i != 0,
            _ => false,
        }
    }
    pub fn as_block_pos(&self) -> [i32; 3] {
        match self {
            JvmVal::BlockPos(p) => *p,
            _ => [0; 3],
        }
    }
    pub fn as_state(&self) -> u32 {
        match self {
            JvmVal::State(s) => *s,
            _ => 0,
        }
    }

    /// Instance method dispatch BY NAME. Calls with a REAL Horizon
    /// implementation route through the current BridgeCtx (set_block / give /
    /// play_sound); everything else logs once and degrades.
    /// Context-free dispatch (standalone mode). Translated code inside
    /// Horizon uses `BridgeCtx::call`, which routes to the real APIs.
    pub fn call(&self, name: &str, args: Vec<JvmVal>) -> JvmVal {
        let _ = args;
        BRIDGE.with(|b| b.borrow_mut().log_unimplemented_inst(self, name));
        JvmVal::Null
    }

    /// Static method dispatch by class+name.
    pub fn call_static(class: &str, name: &str, _args: Vec<JvmVal>) -> JvmVal {
        BRIDGE.with(|b| b.borrow_mut().log_unimplemented_static(class, name));
        JvmVal::Null
    }

    pub fn get_field(&self, _name: &str) -> JvmVal {
        JvmVal::Null
    }
    pub fn set_field(&self, _name: &str, _value: JvmVal) {}
}

// ---------------------------------------------------------------------------
// Bridge: named vanilla calls → Horizon's REAL native APIs
// ---------------------------------------------------------------------------

/// Per-tick context the hook handlers pass down: the game handle plus the
/// dimension the hook fired in. All REAL Horizon calls go through here.
/// The game handle a hook was called with, as a SAFE enum — horizon-client
/// forbids unsafe code, so there are no raw pointers anywhere.
#[allow(dead_code)]
pub enum GameRef<'a, 'g> {
    /// Not running inside Horizon (standalone build of the module).
    Detached(std::marker::PhantomData<&'a mut ()>, std::marker::PhantomData<&'g ()>),
    Game(&'a mut crate::game::Game<'g>),
    Level(&'a mut crate::game::GameLevel<'g>),
}

pub struct BridgeCtx<'a, 'g> {
    pub game: GameRef<'a, 'g>,
    pub dimension: String,
    /// Log of what the bridge did this call (surfaced in diagnostics).
    pub log: Vec<String>,
}

impl<'a, 'g> BridgeCtx<'a, 'g> {
    pub fn new(game: GameRef<'a, 'g>, dimension: impl Into<String>) -> BridgeCtx<'a, 'g> {
        BridgeCtx { game, dimension: dimension.into(), log: Vec::new() }
    }

    /// Detached context for standalone builds and tests.
    pub fn detached() -> BridgeCtx<'a, 'g> {
        BridgeCtx {
            game: GameRef::Detached(std::marker::PhantomData, std::marker::PhantomData),
            dimension: String::new(),
            log: Vec::new(),
        }
    }

    // ---- named dispatch used by translated code (ctx-threaded) ------------

    pub fn call(&mut self, recv: &JvmVal, name: &str, args: Vec<JvmVal>) -> JvmVal {
        match name {
            "setBlock" | "setBlockState" | "setBlockAndUpdate" => {
                let pos = args.first().map(|a| a.as_block_pos()).unwrap_or([0; 3]);
                let state = args.get(1).map(|a| a.as_state()).unwrap_or(0);
                let flags = args.get(2).and_then(|a| match a {
                    JvmVal::Int(i) => Some(*i),
                    _ => Some(3),
                });
                JvmVal::Bool(self.set_block(pos, state, flags.unwrap_or(3)))
            }
            "give" | "giveItem" | "addItem" => {
                let item = args.first().map(|a| a.str_of()).unwrap_or_default();
                let count = args.get(1).and_then(|a| match a {
                    JvmVal::Int(i) => Some(*i as u32),
                    _ => Some(1),
                });
                JvmVal::Bool(self.give(&item, count.unwrap_or(1)))
            }
            "playSound" | "playSeededSound" => {
                let sound = args.first().map(|a| a.str_of()).unwrap_or_default();
                let at = args.get(1).map(|a| a.as_block_pos()).unwrap_or([0; 3]);
                JvmVal::Bool(self.play_sound(
                    &sound,
                    [at[0] as f64, at[1] as f64, at[2] as f64],
                    1.0,
                    1.0,
                ))
            }
            _ => {
                BRIDGE.with(|b| b.borrow_mut().log_unimplemented_inst(recv, name));
                JvmVal::Null
            }
        }
    }

    pub fn call_static(&mut self, class: &str, name: &str, _args: Vec<JvmVal>) -> JvmVal {
        BRIDGE.with(|b| b.borrow_mut().log_unimplemented_static(class, name));
        JvmVal::Null
    }

    pub fn get_field(&mut self, recv: &JvmVal, name: &str) -> JvmVal {
        BRIDGE.with(|b| b.borrow_mut().log_unimplemented_inst(recv, name));
        JvmVal::Null
    }

    /// Block name at coords — REAL world read through Game
    /// (`world.block(x,y,z)` + `res.facts().get(id).name`).
    pub fn block_name_at(&mut self, pos: [i32; 3]) -> String {
        match &mut self.game {
            #[cfg(feature = "j2h-in-horizon")]
            GameRef::Game(g) => {
                let id = g.world.block(pos[0], pos[1], pos[2]);
                g.res.facts().get(id).map(|f| f.name.clone()).unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    /// Set a block BY NAME: registry.default_state -> native_set_block.
    pub fn set_block_by_name(&mut self, pos: [i32; 3], name: &str) -> bool {
        match &mut self.game {
            #[cfg(feature = "j2h-in-horizon")]
            GameRef::Game(g) => {
                let Some(state) = g.pack.assets.registry.default_state(name) else { return false };
                g.native_set_block(&self.dimension, pos, state, 3)
            }
            _ => false,
        }
    }

    /// `Level.setBlock(pos, state, flags)` — REAL: Horizon's
    /// `Game::native_set_block(dimension, pos, state, flags) -> bool`.
    pub fn set_block(&mut self, pos: [i32; 3], state: u32, flags: i32) -> bool {
        self.log.push(format!("set_block {pos:?} state={state} flags={flags}"));
        match &mut self.game {
            GameRef::Game(g) => g.native_set_block(
                &self.dimension,
                pos,
                horizon_world::registry::BlockStateId(state),
                flags,
            ),
            _ => false,
        }
    }

    /// `Player.give(item, count)` — REAL: `Game::native_give(item, count)`.
    pub fn give(&mut self, item: &str, count: u32) -> bool {
        self.log.push(format!("give {item} x{count}"));
        match &mut self.game {
            GameRef::Game(g) => g.native_give(item, count).is_ok(),
            _ => false,
        }
    }

    /// REAL: `GameLevel::native_play_sound(sound, at, source, volume, pitch)`.
    pub fn play_sound(&mut self, sound: &str, at: [f64; 3], volume: f32, pitch: f32) -> bool {
        self.log.push(format!("play_sound {sound} at {at:?}"));
        match &mut self.game {
            GameRef::Level(l) => l.native_play_sound(sound, at, 0, volume, pitch),
            _ => false,
        }
    }

}

// These are filled by the EMITTER with the real `Game` calls when the module
// compiles inside Horizon; standalone (tests) they are no-ops. The emitter
// replaces these bodies — see emit::jvm_rt_prelude().
fn bridge_set_block(_ctx: &mut BridgeCtx, _pos: [i32; 3], _state: u32, _flags: i32) -> bool {
    false
}
fn bridge_give(_ctx: &mut BridgeCtx, _item: &str, _count: u32) -> bool {
    false
}
fn bridge_play_sound(_ctx: &mut BridgeCtx, _s: &str, _at: [f64; 3], _v: f32, _p: f32) -> bool {
    false
}
fn bridge_send_particles(_ctx: &mut BridgeCtx, _o: &str, _at: [f64; 3], _c: i32) -> bool {
    false
}

/// Global call ledger: every unimplemented vanilla call is recorded once so
/// mod authors see exactly what degrades instead of crashes. Thread-local:
/// game hooks run on the game thread.
pub struct Bridge {
    pub seen_static: BTreeMap<(String, String), u64>,
    pub seen_inst: BTreeMap<(String, String), u64>,
}

thread_local! {
    pub static BRIDGE: RefCell<Bridge> = RefCell::new(Bridge {
        seen_static: BTreeMap::new(),
        seen_inst: BTreeMap::new(),
    });
}

impl Bridge {
    fn log_unimplemented_inst(&mut self, recv: &JvmVal, name: &str) {
        let class = match recv {
            JvmVal::Obj(c, _) => c.clone(),
            JvmVal::Str(_) => "java/lang/String".to_string(),
            _ => "<prim>".to_string(),
        };
        let e = self.seen_inst.entry((class.clone(), name.to_string())).or_insert(0);
        if *e == 0 {
            eprintln!("[j2h-bridge] unimplemented instance call {class}.{name} — returning default");
        }
        *e += 1;
    }
    fn log_unimplemented_static(&mut self, class: &str, name: &str) {
        let e = self.seen_static.entry((class.to_string(), name.to_string())).or_insert(0);
        if *e == 0 {
            eprintln!("[j2h-bridge] unimplemented static call {class}.{name} — returning default");
        }
        *e += 1;
    }
}

// ---------------------------------------------------------------------------
// Emitted-code helpers (referenced by the translator output)
// ---------------------------------------------------------------------------

/// `invokedynamic makeConcatWithConstants` unfolding.
pub fn jvm_concat(parts: Vec<String>) -> JvmVal {
    JvmVal::string(parts.concat())
}

/// `a instanceof B` — degrades to a name check on the handle.
pub fn jvm_instanceof(v: &JvmVal, class: &str) -> JvmVal {
    JvmVal::Bool(matches!(v, JvmVal::Obj(c, _) if *c == class))
}

/// Null identity checks (if_acmp against null).
pub fn jvm_is_null(v: &JvmVal) -> bool {
    matches!(v, JvmVal::Null)
}

// ---------------------------------------------------------------------------
// Context stack: hook handlers push the BridgeCtx so named dispatch inside
// translated code can reach the real Horizon APIs.
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// Emitted-code helpers (referenced by the translator output)
// ---------------------------------------------------------------------------

/// `newarray` / `anewarray`: bytes for networking, generic slot list otherwise.
pub fn jvm_array_new(len: i32) -> JvmVal {
    JvmVal::Bytes(Rc::new(RefCell::new(vec![0u8; len.max(0) as usize])))
}
pub fn jvm_array_load(arr: &JvmVal, idx: i32) -> JvmVal {
    match arr {
        JvmVal::Bytes(b) => b.borrow().get(idx as usize).map(|&v| JvmVal::Int(v as i32)).unwrap_or(JvmVal::Null),
        _ => JvmVal::Null,
    }
}
pub fn jvm_array_store(arr: &JvmVal, idx: i32, v: JvmVal) {
    if let (JvmVal::Bytes(b), JvmVal::Int(n)) = (arr, v) {
        let mut b = b.borrow_mut();
        if let Some(slot) = b.get_mut(idx as usize) {
            *slot = n as u8;
        }
    }
}
pub fn jvm_array_length(arr: &JvmVal) -> JvmVal {
    match arr {
        JvmVal::Bytes(b) => JvmVal::Int(b.borrow().len() as i32),
        _ => JvmVal::Int(0),
    }
}

/// `if_acmpeq` on handles.
pub fn jvm_ref_eq(l: &JvmVal, r: &JvmVal) -> bool {
    match (l, r) {
        (JvmVal::Obj(_, a), JvmVal::Obj(_, b)) => a == b,
        (JvmVal::Null, JvmVal::Null) => true,
        (JvmVal::Str(a), JvmVal::Str(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}

/// `athrow` — logged and swallowed: mods using exceptions for control flow
/// degrade to their fall-through path instead of aborting the game.
pub fn jvm_throw(v: &JvmVal) {
    eprintln!("[j2h-bridge] swallowed athrow: {}", v.str_of());
}

// ---------------------------------------------------------------------------
// Numeric helpers: the translator emits ALL arithmetic through these, so any
// mix of JvmVal kinds compiles and behaves like Java's primitive model.
// ---------------------------------------------------------------------------

fn as_f64(v: &JvmVal) -> f64 {
    match v {
        JvmVal::Int(i) => *i as f64,
        JvmVal::Long(l) => *l as f64,
        JvmVal::Float(f) => *f as f64,
        JvmVal::Double(d) => *d,
        JvmVal::Bool(b) => *b as i32 as f64,
        _ => 0.0,
    }
}

fn from_f64_f32(x: f64) -> JvmVal {
    JvmVal::Float(x as f32)
}

fn from_f64_f64(x: f64) -> JvmVal {
    JvmVal::Double(x)
}

fn from_f64_int(x: f64) -> JvmVal {
    JvmVal::Int(x as i32)
}

fn from_f64_long(x: f64) -> JvmVal {
    JvmVal::Long(x as i64)
}

/// Binary op with Java numeric promotion. `int_result` marks ops whose Java
/// result is int/long even on float inputs (shifts).
pub fn jvm_bin(op: &str, l: &JvmVal, r: &JvmVal) -> JvmVal {
    let wide = matches!(l, JvmVal::Long(_)) || matches!(r, JvmVal::Long(_));
    let float = matches!(l, JvmVal::Float(_) | JvmVal::Double(_))
        || matches!(r, JvmVal::Float(_) | JvmVal::Double(_));
    match op {
        "+" | "-" | "*" | "/" | "%" => {
            let (a, b) = (as_f64(l), as_f64(r));
            let x = match op {
                "+" => a + b,
                "-" => a - b,
                "*" => a * b,
                "/" => {
                    if b == 0.0 {
                        // Java: int div by zero throws; floats produce inf/nan.
                        if float { f64::NAN } else { return JvmVal::Int(0); }
                    } else {
                        a / b
                    }
                }
                _ => {
                    if b == 0.0 {
                        return JvmVal::Int(0);
                    }
                    a % b
                }
            };
            if float {
                if matches!(l, JvmVal::Double(_)) || matches!(r, JvmVal::Double(_)) {
                    from_f64_f64(x)
                } else {
                    from_f64_f32(x)
                }
            } else if wide {
                from_f64_long(x)
            } else {
                from_f64_int(x)
            }
        }
        "&" | "|" | "^" => {
            let (a, b) = (int_of(l), int_of(r));
            let x = match op {
                "&" => a & b,
                "|" => a | b,
                _ => a ^ b,
            };
            if wide { JvmVal::Long(x) } else { JvmVal::Int(x as i32) }
        }
        "<<" | ">>" | ">>>" => {
            let sh = match r {
                JvmVal::Int(i) => (*i as u64) & 63,
                JvmVal::Long(i) => (*i as u64) & 63,
                _ => 0,
            };
            if wide {
                let a = match l {
                    JvmVal::Long(i) => *i,
                    _ => int_of(l),
                };
                let x = match op {
                    "<<" => ((a as u64) << sh) as i64,
                    ">>" => a >> sh,
                    _ => ((a as u64) >> sh) as i64,
                };
                JvmVal::Long(x)
            } else {
                let a = match l {
                    JvmVal::Int(i) => *i,
                    _ => int_of(l) as i32,
                };
                let s = (sh & 31) as u32;
                let x = match op {
                    "<<" => ((a as u32) << s) as i32,
                    ">>" => a >> s,
                    _ => ((a as u32) >> s) as i32,
                };
                JvmVal::Int(x)
            }
        }
        _ => JvmVal::Null,
    }
}

fn int_of(v: &JvmVal) -> i64 {
    match v {
        JvmVal::Int(i) => *i as i64,
        JvmVal::Long(l) => *l,
        JvmVal::Bool(b) => *b as i64,
        JvmVal::Float(f) => *f as i64,
        JvmVal::Double(d) => *d as i64,
        _ => 0,
    }
}

/// Numeric negation.
pub fn jvm_neg(v: &JvmVal) -> JvmVal {
    jvm_bin("-", &JvmVal::Int(0), v)
}

/// Casts (`i2l`, `f2i`, `i2b`, ...): kind is the target java letter.
pub fn jvm_conv(v: &JvmVal, kind: &str) -> JvmVal {
    let x = as_f64(v);
    match kind {
        "I" => JvmVal::Int(x as i32),
        "J" => JvmVal::Long(x as i64),
        "F" => JvmVal::Float(x as f32),
        "D" => JvmVal::Double(x),
        "B" => JvmVal::Int(x as i8 as i32),
        "C" => JvmVal::Int(x as u16 as i32),
        "S" => JvmVal::Int(x as i16 as i32),
        _ => v.clone(),
    }
}

/// `iinc`: local += delta.
pub fn jvm_iinc(v: &JvmVal, delta: i32) -> JvmVal {
    jvm_bin("+", v, &JvmVal::Int(delta))
}

/// Java `lcmp`/`fcmpl`/`dcmpg` unified.
pub fn jvm_cmp(l: &JvmVal, r: &JvmVal) -> JvmVal {
    let (a, b) = (as_f64(l), as_f64(r));
    JvmVal::Int(if a < b { -1 } else if a > b { 1 } else { 0 })
}



// --- translated mod logic ---

// --- from com/simibubi/create/foundation/mixin/ArmorTrimMixin.class :: getColorPaletteSuffix (java static)
pub fn get_color_palette_suffix(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/AssertionError"));
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/ArmorTrimMixin.class :: create$swapTexturesForCardboardTrimsInner (java instance)
pub fn create_swap_textures_for_cardboard_trims_inner(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_ref_eq(&p0, &ctx.call_static("com/simibubi/create/content/equipment/armor/AllArmorMaterials", "CARDBOARD", vec![])) { pc = 28; continue 'jvm; }
                pc = 8;
            }
            7 => {
    let arg_tmp_0 = ctx.call_static("java/lang/Boolean", "valueOf", vec![JvmVal::Int(1)]);
                p1.call("setReturnValue", vec![p_this.get_field("create$textureCardboard").call("apply", vec![arg_tmp_0.clone(), p0.clone()])]);
                pc = 28;
            }
            28 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/ArmorTrimMixin.class :: create$swapTexturesForCardboardTrimsOuter (java instance)
pub fn create_swap_textures_for_cardboard_trims_outer(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_ref_eq(&p0, &ctx.call_static("com/simibubi/create/content/equipment/armor/AllArmorMaterials", "CARDBOARD", vec![])) { pc = 28; continue 'jvm; }
                pc = 8;
            }
            7 => {
    let arg_tmp_0 = ctx.call_static("java/lang/Boolean", "valueOf", vec![JvmVal::Int(0)]);
                p1.call("setReturnValue", vec![p_this.get_field("create$textureCardboard").call("apply", vec![arg_tmp_0.clone(), p0.clone()])]);
                pc = 28;
            }
            28 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/ArmorTrimMixin.class :: lambda$new$0 (java instance)
pub fn lambda_new_0(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p_this.get_field("pattern").call("value", vec![]).call("assetId", vec![]).call("getPath", vec![])).clone();
                local_4 = (ctx.call_static("com/simibubi/create/foundation/mixin/ArmorTrimMixin", "getColorPaletteSuffix", vec![p_this.get_field("material"), p1.clone()])).clone();
                if jvm_cmp(&p0.call("booleanValue", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 42; continue 'jvm; }
                pc = 39;
            }
            37 => {
                pc = 44;
            }
            42 => {
                pc = 46;
            }
            44 => {
                return ctx.call_static("com/simibubi/create/Create", "asResource", vec![JvmVal::string(format!(""))]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/BeehiveBlockMixin.class :: create$dontGetAngryAtDeployers (java static)
pub fn create_dont_get_angry_at_deployers(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p0, &JvmVal::Int(0)).as_i32() != 0 { pc = 11; continue 'jvm; }
                pc = 5;
            }
            4 => {
                if jvm_cmp(&jvm_instanceof(&p1, "com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer"), &JvmVal::Int(0)).as_i32() == 0 { pc = 15; continue 'jvm; }
                pc = 12;
            }
            11 => {
                pc = 16;
            }
            15 => {
                pc = 16;
            }
            16 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/BlockItemMixin.class :: create$fixDeployerPlacement (java instance)
pub fn create_fix_deployer_placement(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p0.call("getLevel", vec![]).call("getBlockState", vec![p0.call("create$getHitResult", vec![]).call("getBlockPos", vec![])])).clone();
                if jvm_cmp(&local_3.call("canBeReplaced", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 44; continue 'jvm; }
                pc = 28;
            }
            27 => {
                if jvm_cmp(&jvm_instanceof(&p0.call("getPlayer", vec![]), "com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer"), &JvmVal::Int(0)).as_i32() == 0 { pc = 44; continue 'jvm; }
                pc = 38;
            }
            37 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "PASS", vec![]);
                p1.call("setReturnValue", vec![arg_tmp_0.clone()]);
                pc = 44;
            }
            44 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/BlockMixin.class :: create$handlePopResource (java static)
pub fn create_handle_pop_resource(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (ctx.call_static("com/simibubi/create/content/kinetics/deployer/DeployerHandler", "CAPTURED_BLOCK_DROPS_VIEW", vec![]).call("get", vec![p1.call("blockPosition", vec![])])).clone();
                if jvm_is_null(&local_2) { pc = 33; continue 'jvm; }
                pc = 21;
            }
            20 => {
                return JvmVal::Int(0);
            }
            33 => {
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/BuiltInRegistriesMixin.class :: create$ourRegistriesAreNotEmpty (java static)
pub fn create_our_registries_are_not_empty(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (JvmVal::Null).clone();
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), p0.clone());
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(1).as_i32(), local_3.clone());
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/BuiltInRegistriesMixin.class :: lambda$create$ourRegistriesAreNotEmpty$0 (java static)
pub fn lambda_create_our_registries_are_not_empty_0(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p1.call("key", vec![]).call("location", vec![]).call("getNamespace", vec![]).call("equals", vec![JvmVal::string("create")]), &JvmVal::Int(0)).as_i32() != 0 { pc = 27; continue 'jvm; }
                pc = 21;
            }
            20 => {
                p0.call("accept", vec![p1.clone()]);
                pc = 27;
            }
            27 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/CustomItemUseEffectsMixin.class :: create$onShouldTriggerUseEffects (java instance)
pub fn create_on_should_trigger_use_effects(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (p_this.call("getUseItem", vec![])).clone();
                local_3 = (local_2.call("getItem", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_3, "com/simibubi/create/foundation/item/CustomUseEffectsItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 57; continue 'jvm; }
                pc = 18;
            }
            17 => {
                local_4 = (local_3).clone();
                local_5 = (local_4.call("shouldTriggerUseEffects", vec![local_2.clone(), p_this.clone()])).clone();
                if jvm_ref_eq(&local_5, &ctx.call_static("net/createmod/catnip/data/TriState", "DEFAULT", vec![])) { pc = 57; continue 'jvm; }
                pc = 46;
            }
            45 => {
    let arg_tmp_0 = ctx.call_static("java/lang/Boolean", "valueOf", vec![local_5.call("getValue", vec![])]);
                p0.call("setReturnValue", vec![arg_tmp_0.clone()]);
                pc = 57;
            }
            57 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/CustomItemUseEffectsMixin.class :: create$onTriggerUseEffects (java instance)
pub fn create_on_trigger_use_effects(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (p0.call("getItem", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_4, "com/simibubi/create/foundation/item/CustomUseEffectsItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 45; continue 'jvm; }
                pc = 16;
            }
            14 => {
                local_5 = (local_4).clone();
                if jvm_cmp(&local_5.call("triggerUseEffects", vec![p0.clone(), p_this.clone(), p1.clone(), p_this.get_field("random")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 45; continue 'jvm; }
                pc = 42;
            }
            41 => {
                p2.call("cancel", vec![]);
                pc = 45;
            }
            45 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/EnchantedCountIncreaseFunctionMixin.class :: create$crushingWheelLooting (java instance)
pub fn create_crushing_wheel_looting(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/level/storage/loot/parameters/LootContextParams", "DAMAGE_SOURCE", vec![]);
                local_4 = (p1.call("getParamOrNull", vec![arg_tmp_0.clone()])).clone();
                if jvm_is_null(&local_4) { pc = 71; continue 'jvm; }
                pc = 19;
            }
            17 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDamageTypes", "CRUSH", vec![]);
                if jvm_cmp(&local_4.call("is", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 71; continue 'jvm; }
                pc = 29;
            }
            28 => {
                local_5 = (JvmVal::Int(2)).clone();
                local_6 = (jvm_bin("*", &jvm_conv(&local_5, "F"), &p_this.get_field("value").call("getFloat", vec![p1.clone()]))).clone();
    let arg_tmp_0 = ctx.call_static("java/lang/Math", "round", vec![local_6.clone()]);
                p0.call("grow", vec![arg_tmp_0.clone()]);
                if jvm_cmp(&p_this.call("hasLimit", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 71; continue 'jvm; }
                pc = 64;
            }
            63 => {
                p0.call("limitSize", vec![p_this.get_field("limit")]);
                pc = 71;
            }
            71 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/EntityMixin.class :: create$onFireImmune (java instance)
pub fn create_on_fire_immune(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p_this.call("getPersistentData", vec![]).call("getBoolean", vec![JvmVal::string("CreateFireImmune")]), &JvmVal::Int(0)).as_i32() != 0 { pc = 19; continue 'jvm; }
                pc = 16;
            }
            15 => {
                if jvm_cmp(&p0, &JvmVal::Int(0)).as_i32() == 0 { pc = 23; continue 'jvm; }
                pc = 20;
            }
            19 => {
                pc = 24;
            }
            23 => {
                pc = 24;
            }
            24 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/EntityMixin.class :: create$fixSeatBouncing (java instance)
pub fn create_fix_seat_bouncing(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p_this.call("getBlockStateOn", vec![]).call("getBlock", vec![]), "com/simibubi/create/content/contraptions/actors/seat/SeatBlock"), &JvmVal::Int(0)).as_i32() == 0 { pc = 20; continue 'jvm; }
                pc = 14;
            }
            13 => {
                pc = 21;
            }
            20 => {
                pc = 21;
            }
            21 => {
                return p0;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/ItemStackMixin.class :: create$migrateOldClipboardComponents (java instance)
pub fn create_migrate_old_clipboard_components(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/mixin/ItemStackMixin", "create$CLIPBOARD_ID", vec![]);
                if jvm_cmp(&ctx.call_static("net/minecraft/core/registries/BuiltInRegistries", "ITEM", vec![]).call("getKey", vec![p0.call("asItem", vec![])]).call("equals", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 24; continue 'jvm; }
                pc = 24;
            }
            23 => {
                return;
            }
            24 => {
                local_5 = (ctx.call_static("com/simibubi/create/content/equipment/clipboard/ClipboardContent", "EMPTY", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLIPBOARD_PAGES", vec![]);
                local_5 = (ctx.call_static("com/simibubi/create/foundation/mixin/ItemStackMixin", "create$migrateComponent", vec![local_5.clone(), p2.clone(), arg_tmp_0.clone(), JvmVal::Null])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLIPBOARD_TYPE", vec![]);
                local_5 = (ctx.call_static("com/simibubi/create/foundation/mixin/ItemStackMixin", "create$migrateComponent", vec![local_5.clone(), p2.clone(), arg_tmp_0.clone(), JvmVal::Null])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLIPBOARD_READ_ONLY", vec![]);
                local_5 = (ctx.call_static("com/simibubi/create/foundation/mixin/ItemStackMixin", "create$migrateComponent", vec![local_5.clone(), p2.clone(), arg_tmp_0.clone(), JvmVal::Null])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLIPBOARD_COPIED_VALUES", vec![]);
                local_5 = (ctx.call_static("com/simibubi/create/foundation/mixin/ItemStackMixin", "create$migrateComponent", vec![local_5.clone(), p2.clone(), arg_tmp_0.clone(), JvmVal::Null])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLIPBOARD_PREVIOUSLY_OPENED_PAGE", vec![]);
                local_5 = (ctx.call_static("com/simibubi/create/foundation/mixin/ItemStackMixin", "create$migrateComponent", vec![local_5.clone(), p2.clone(), arg_tmp_0.clone(), JvmVal::Null])).clone();
                if jvm_ref_eq(&local_5, &ctx.call_static("com/simibubi/create/content/equipment/clipboard/ClipboardContent", "EMPTY", vec![])) { pc = 127; continue 'jvm; }
                pc = 118;
            }
            117 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLIPBOARD_CONTENT", vec![]);
                pc = 127;
            }
            127 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/ItemStackMixin.class :: create$migrateComponent (java static)
pub fn create_migrate_component(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (p1.call("get", vec![p2.clone()])).clone();
                if jvm_is_null(&local_4) { pc = 31; continue 'jvm; }
                pc = 13;
            }
            12 => {
                p0 = (p3.call("apply", vec![p0.clone(), local_4.clone()])).clone();
                pc = 32;
            }
            31 => {
                return p0;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/ItemStackMixin.class :: lambda$create$migrateOldClipboardComponents$0 (java static)
pub fn lambda_create_migrate_old_clipboard_components_0(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p0.call("setReadOnly", vec![JvmVal::Int(1)]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/LavaSwimmingMixin.class :: create$onLavaTravel (java instance)
pub fn create_on_lava_travel(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (ctx.call_static("com/simibubi/create/content/equipment/armor/DivingBootsItem", "getWornItem", vec![p_this.clone()])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "NETHERITE_DIVING_BOOTS", vec![]).call("isIn", vec![local_3.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 33; continue 'jvm; }
                pc = 16;
            }
            15 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/armor/DivingBootsItem", "getMovementMultiplier", vec![p_this.clone()]);
                p_this.call("setDeltaMovement", vec![p_this.call("getDeltaMovement", vec![]).call("multiply", vec![arg_tmp_0.clone()])]);
                pc = 33;
            }
            33 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/MapItemSavedDataMixin.class :: create$onLoad (java static)
pub fn create_on_load(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p2.call("getReturnValue", vec![])).clone();
                local_4 = (local_3).clone();
                local_5 = (p0.call("getList", vec![JvmVal::string("create:stations"), JvmVal::Int(10)])).clone();
                local_6 = (JvmVal::Int(0)).clone();
                pc = 29;
            }
            27 => {
                if jvm_cmp(&local_6, &local_5.call("size", vec![])).as_i32() >= 0 { pc = 65; continue 'jvm; }
                pc = 39;
            }
            37 => {
                local_7 = (ctx.call_static("com/simibubi/create/content/trains/station/StationMarker", "load", vec![local_5.call("getCompound", vec![local_6.clone()]), p1.clone()])).clone();
                local_4.call("addStationMarker", vec![local_7.clone()]);
                local_6 = jvm_iinc(&local_6, 1);
                pc = 27;
            }
            65 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/MapItemSavedDataMixin.class :: create$onSave (java instance)
pub fn create_on_save(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (JvmVal::obj("net/minecraft/nbt/ListTag")).clone();
                local_5 = (p_this.get_field("create$stationMarkers").call("values", vec![]).call("iterator", vec![])).clone();
                pc = 27;
            }
            25 => {
                if jvm_cmp(&local_5.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 62; continue 'jvm; }
                pc = 37;
            }
            35 => {
                local_6 = (local_5.call("next", vec![])).clone();
                pc = 25;
            }
            62 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/MapItemSavedDataMixin.class :: addStationMarker (java instance)
pub fn add_station_marker(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (jvm_bin("<<", &JvmVal::Int(1), &p_this.get_field("scale"))).clone();
                local_3 = (jvm_bin("/", &jvm_conv(&jvm_bin("-", &p0.call("getTarget", vec![]).call("getX", vec![]), &p_this.get_field("centerX")), "F"), &jvm_conv(&local_2, "F"))).clone();
                local_4 = (jvm_bin("/", &jvm_conv(&jvm_bin("-", &p0.call("getTarget", vec![]).call("getZ", vec![]), &p_this.get_field("centerZ")), "F"), &jvm_conv(&local_2, "F"))).clone();
                if jvm_cmp(&jvm_cmp(&local_3, &JvmVal::Float(-63_f32)), &JvmVal::Int(0)).as_i32() < 0 { pc = 87; continue 'jvm; }
                pc = 65;
            }
            64 => {
                if jvm_cmp(&jvm_cmp(&local_3, &JvmVal::Float(63_f32)), &JvmVal::Int(0)).as_i32() > 0 { pc = 87; continue 'jvm; }
                pc = 73;
            }
            71 => {
                if jvm_cmp(&jvm_cmp(&local_4, &JvmVal::Float(-63_f32)), &JvmVal::Int(0)).as_i32() < 0 { pc = 87; continue 'jvm; }
                pc = 81;
            }
            79 => {
                if jvm_cmp(&jvm_cmp(&local_4, &JvmVal::Float(63_f32)), &JvmVal::Int(0)).as_i32() <= 0 { pc = 96; continue 'jvm; }
                pc = 88;
            }
            87 => {
                p_this.call("removeDecoration", vec![p0.call("getId", vec![])]);
                return;
            }
            96 => {
                local_5 = (jvm_conv(&jvm_conv(&jvm_bin("+", &jvm_bin("*", &local_3, &JvmVal::Float(2_f32)), &JvmVal::Float(0.5_f32)), "I"), "B")).clone();
                local_6 = (jvm_conv(&jvm_conv(&jvm_bin("+", &jvm_bin("*", &local_4, &JvmVal::Float(2_f32)), &JvmVal::Float(0.5_f32)), "I"), "B")).clone();
    let arg_tmp_0 = ctx.call_static("java/util/Optional", "of", vec![p0.call("getName", vec![])]);
                local_7 = (ctx.call_static("com/simibubi/create/content/trains/station/StationMarker", "createStationDecoration", vec![local_5.clone(), local_6.clone(), arg_tmp_0.clone()])).clone();
                local_8 = (p_this.get_field("decorations").call("put", vec![p0.call("getId", vec![]), local_7.clone()])).clone();
                if jvm_cmp(&local_7.call("equals", vec![local_8.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 230; continue 'jvm; }
                pc = 165;
            }
            163 => {
                if jvm_is_null(&local_8) { pc = 197; continue 'jvm; }
                pc = 170;
            }
            168 => {
                if jvm_cmp(&local_8.call("type", vec![]).call("value", vec![]).call("trackCount", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 197; continue 'jvm; }
                pc = 188;
            }
            187 => {
                // putfield: jvm_bin("-", &p_this.get_field("trackedDecorationCount"), &JvmVal::Int(1)) handled by the bridge object model
                pc = 199;
            }
            197 => {
                if jvm_cmp(&local_7.call("type", vec![]).call("value", vec![]).call("trackCount", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 226; continue 'jvm; }
                pc = 217;
            }
            216 => {
                // putfield: jvm_bin("+", &p_this.get_field("trackedDecorationCount"), &JvmVal::Int(1)) handled by the bridge object model
                pc = 227;
            }
            226 => {
                p_this.call("setDecorationsDirty", vec![]);
                pc = 230;
            }
            230 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/MapItemSavedDataMixin.class :: removeDecoration (java instance)
pub fn remove_decoration(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/AssertionError"));
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/MapItemSavedDataMixin.class :: setDecorationsDirty (java instance)
pub fn set_decorations_dirty(ctx: &mut BridgeCtx, p_this: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/AssertionError"));
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/MapItemSavedDataMixin.class :: isTrackedCountOverLimit (java instance)
pub fn is_tracked_count_over_limit(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/AssertionError"));
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/MapItemSavedDataMixin.class :: toggleStation (java instance)
pub fn toggle_station(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (jvm_bin("+", &jvm_conv(&p1.call("getX", vec![]), "D"), &JvmVal::Double(0.5_f64))).clone();
                local_6 = (jvm_bin("+", &jvm_conv(&p1.call("getZ", vec![]), "D"), &JvmVal::Double(0.5_f64))).clone();
                local_8 = (jvm_bin("<<", &JvmVal::Int(1), &p_this.get_field("scale"))).clone();
                local_9 = (jvm_bin("/", &jvm_bin("-", &local_4, &jvm_conv(&p_this.get_field("centerX"), "D")), &jvm_conv(&local_8, "D"))).clone();
                local_11 = (jvm_bin("/", &jvm_bin("-", &local_6, &jvm_conv(&p_this.get_field("centerZ"), "D")), &jvm_conv(&local_8, "D"))).clone();
                if jvm_cmp(&jvm_cmp(&local_9, &JvmVal::Double(-63_f64)), &JvmVal::Int(0)).as_i32() < 0 { pc = 94; continue 'jvm; }
                pc = 69;
            }
            67 => {
                if jvm_cmp(&jvm_cmp(&local_9, &JvmVal::Double(63_f64)), &JvmVal::Int(0)).as_i32() > 0 { pc = 94; continue 'jvm; }
                pc = 78;
            }
            76 => {
                if jvm_cmp(&jvm_cmp(&local_11, &JvmVal::Double(-63_f64)), &JvmVal::Int(0)).as_i32() < 0 { pc = 94; continue 'jvm; }
                pc = 87;
            }
            85 => {
                if jvm_cmp(&jvm_cmp(&local_11, &JvmVal::Double(63_f64)), &JvmVal::Int(0)).as_i32() <= 0 { pc = 96; continue 'jvm; }
                pc = 95;
            }
            94 => {
                return JvmVal::Int(0);
            }
            96 => {
                local_13 = (ctx.call_static("com/simibubi/create/content/trains/station/StationMarker", "fromWorld", vec![p0.clone(), p1.clone()])).clone();
                if !jvm_is_null(&local_13) { pc = 110; continue 'jvm; }
                pc = 109;
            }
            108 => {
                return JvmVal::Int(0);
            }
            110 => {
                if jvm_cmp(&p_this.get_field("create$stationMarkers").call("remove", vec![local_13.call("getId", vec![]), local_13.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 140; continue 'jvm; }
                pc = 130;
            }
            129 => {
                p_this.call("removeDecoration", vec![local_13.call("getId", vec![])]);
                return JvmVal::Int(1);
            }
            140 => {
                if jvm_cmp(&p_this.call("isTrackedCountOverLimit", vec![JvmVal::Int(256)]), &JvmVal::Int(0)).as_i32() != 0 { pc = 158; continue 'jvm; }
                pc = 151;
            }
            150 => {
                p_this.call("addStationMarker", vec![local_13.clone()]);
                return JvmVal::Int(1);
            }
            158 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/MapItemSavedDataMixin.class :: create$onCheckBanners (java instance)
pub fn create_on_check_banners(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                p_this.call("create$checkStations", vec![p0.clone(), p1.clone(), p2.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/MapItemSavedDataMixin.class :: create$checkStations (java instance)
pub fn create_check_stations(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (p_this.get_field("create$stationMarkers").call("values", vec![]).call("iterator", vec![])).clone();
                local_5 = (JvmVal::obj("java/util/ArrayList")).clone();
                pc = 27;
            }
            25 => {
                if jvm_cmp(&local_4.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 142; continue 'jvm; }
                pc = 37;
            }
            35 => {
                local_6 = (local_4.call("next", vec![])).clone();
                if jvm_cmp(&local_6.call("getTarget", vec![]).call("getX", vec![]), &p1).as_i32() != 0 { pc = 139; continue 'jvm; }
                pc = 61;
            }
            59 => {
                if jvm_cmp(&local_6.call("getTarget", vec![]).call("getZ", vec![]), &p2).as_i32() != 0 { pc = 139; continue 'jvm; }
                pc = 72;
            }
            71 => {
                local_7 = (ctx.call_static("com/simibubi/create/content/trains/station/StationMarker", "fromWorld", vec![p0.clone(), local_6.call("getSource", vec![])])).clone();
                if jvm_cmp(&local_6.call("equals", vec![local_7.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 139; continue 'jvm; }
                pc = 94;
            }
            92 => {
                local_4.call("remove", vec![]);
                p_this.call("removeDecoration", vec![local_6.call("getId", vec![])]);
                if jvm_is_null(&local_7) { pc = 139; continue 'jvm; }
                pc = 115;
            }
            113 => {
                if jvm_cmp(&local_6.call("getTarget", vec![]).call("equals", vec![local_7.call("getTarget", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 139; continue 'jvm; }
                pc = 131;
            }
            129 => {
                pc = 142;
            }
            139 => {
                pc = 25;
            }
            142 => {
                local_6 = (local_5.call("iterator", vec![])).clone();
                pc = 153;
            }
            151 => {
                if jvm_cmp(&local_6.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 182; continue 'jvm; }
                pc = 163;
            }
            161 => {
                local_7 = (local_6.call("next", vec![])).clone();
                p_this.call("addStationMarker", vec![local_7.clone()]);
                pc = 151;
            }
            182 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/MobMixin.class :: create$mobRidingContraptionsMaintainTheirAttackBox (java instance)
pub fn create_mob_riding_contraptions_maintain_their_attack_box(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p0, "com/simibubi/create/content/contraptions/AbstractContraptionEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 11; continue 'jvm; }
                pc = 8;
            }
            7 => {
                pc = 12;
            }
            11 => {
                pc = 12;
            }
            12 => {
                return p0;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/PlayerMixin.class :: create$playerHidingAsBoxIsCrouchingNotSwimming (java instance)
pub fn create_player_hiding_as_box_is_crouching_not_swimming(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p0, &JvmVal::Int(0)).as_i32() != 0 { pc = 18; continue 'jvm; }
                pc = 5;
            }
            4 => {
                if !jvm_ref_eq(&p1, &ctx.call_static("net/minecraft/world/entity/Pose", "CROUCHING", vec![])) { pc = 22; continue 'jvm; }
                pc = 12;
            }
            11 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/armor/CardboardArmorHandler", "testForStealth", vec![p_this.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 22; continue 'jvm; }
                pc = 19;
            }
            18 => {
                pc = 23;
            }
            22 => {
                pc = 23;
            }
            23 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/PlayerMixin.class :: pretendNotPassenger (java instance)
pub fn pretend_not_passenger(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (ctx.call_static("com/simibubi/create/infrastructure/config/AllConfigs", "server", vec![]).get_field("kinetics").get_field("syncPlayerPickupHitboxWithContraptionHitbox").call("get", vec![]).call("booleanValue", vec![])).clone();
                if jvm_cmp(&p0, &JvmVal::Int(0)).as_i32() == 0 { pc = 39; continue 'jvm; }
                pc = 24;
            }
            23 => {
                if jvm_cmp(&local_2, &JvmVal::Int(0)).as_i32() != 0 { pc = 39; continue 'jvm; }
                pc = 28;
            }
            27 => {
                if jvm_cmp(&jvm_instanceof(&p_this.call("getVehicle", vec![]), "com/simibubi/create/content/contraptions/AbstractContraptionEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 39; continue 'jvm; }
                pc = 38;
            }
            37 => {
                return JvmVal::Int(0);
            }
            39 => {
                return p0;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/ProjectileUtilMixin.class :: create$interactWithEntitiesOnContraptions (java static)
pub fn create_interact_with_entities_on_contraptions(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), p0.clone());
                if jvm_cmp(&p1.call("call", vec![jvm_array_new(0)]).call("booleanValue", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 33; continue 'jvm; }
                pc = 24;
            }
            23 => {
                if jvm_cmp(&jvm_instanceof(&p0.call("getRootVehicle", vec![]), "com/simibubi/create/content/contraptions/AbstractContraptionEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 37; continue 'jvm; }
                pc = 34;
            }
            33 => {
                pc = 38;
            }
            37 => {
                pc = 38;
            }
            38 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/SmithingMenuMixin.class :: create$awardAdvancementWhenTrimmingCardboardArmor (java instance)
pub fn create_award_advancement_when_trimming_cardboard_armor(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "CARDBOARD_HELMET", vec![]).call("isIn", vec![p1.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 40; continue 'jvm; }
                pc = 13;
            }
            10 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "CARDBOARD_CHESTPLATE", vec![]).call("isIn", vec![p1.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 40; continue 'jvm; }
                pc = 23;
            }
            20 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "CARDBOARD_LEGGINGS", vec![]).call("isIn", vec![p1.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 40; continue 'jvm; }
                pc = 33;
            }
            30 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "CARDBOARD_BOOTS", vec![]).call("isIn", vec![p1.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 47; continue 'jvm; }
                pc = 43;
            }
            40 => {
                ctx.call_static("com/simibubi/create/foundation/advancement/AllAdvancements", "CARDBOARD_ARMOR_TRIM", vec![]).call("awardTo", vec![p0.clone()]);
                pc = 47;
            }
            47 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/SmithingMenuMixin.class :: create$preventUnbreakingOnBacktanks (java instance)
pub fn create_prevent_unbreaking_on_backtanks(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "COPPER_BACKTANK", vec![]).call("isIn", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 20; continue 'jvm; }
                pc = 13;
            }
            10 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "NETHERITE_BACKTANK", vec![]).call("isIn", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 54; continue 'jvm; }
                pc = 23;
            }
            20 => {
                local_2 = (JvmVal::obj("net/minecraft/world/item/enchantment/ItemEnchantments$Mutable")).clone();
                p0.call("removeIf", vec![JvmVal::Null]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/component/DataComponents", "ENCHANTMENTS", vec![]);
                pc = 55;
            }
            54 => {
                return p0;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/SmithingMenuMixin.class :: lambda$create$preventUnbreakingOnBacktanks$0 (java static)
pub fn lambda_create_prevent_unbreaking_on_backtanks_0(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p0.call("supportsEnchantment", vec![p1.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 12; continue 'jvm; }
                pc = 9;
            }
            8 => {
                pc = 13;
            }
            12 => {
                pc = 13;
            }
            13 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/WaterWheelFluidSpreadMixin.class :: create$canPassThroughOnWaterWheel (java instance)
pub fn create_can_pass_through_on_water_wheel(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal, p5: JvmVal, p6: JvmVal, p7: JvmVal, p8: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut p5: JvmVal = p5.clone();
    let mut p6: JvmVal = p6.clone();
    let mut p7: JvmVal = p7.clone();
    let mut p8: JvmVal = p8.clone();
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_ref_eq(&p4.call("getAxis", vec![]), &ctx.call_static("net/minecraft/core/Direction$Axis", "Y", vec![])) { pc = 12; continue 'jvm; }
                pc = 12;
            }
            11 => {
                return;
            }
            12 => {
                local_10 = (p2.call("below", vec![])).clone();
                local_11 = (p0.call("getBlockState", vec![local_10.clone()])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "WATER_WHEEL_STRUCTURAL", vec![]).call("has", vec![local_11.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 79; continue 'jvm; }
                pc = 42;
            }
            39 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "WATER_WHEEL_STRUCTURAL", vec![]).call("get", vec![]).call("stillValid", vec![p0.clone(), local_10.clone(), local_11.clone(), JvmVal::Int(0)]), &JvmVal::Int(0)).as_i32() == 0 { pc = 91; continue 'jvm; }
                pc = 61;
            }
            60 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/waterwheel/WaterWheelStructuralBlock", "getMaster", vec![p0.clone(), local_10.clone(), local_11.clone()]);
                local_11 = (p0.call("getBlockState", vec![arg_tmp_0.clone()])).clone();
                pc = 91;
            }
            79 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "WATER_WHEEL", vec![]).call("has", vec![local_11.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 91; continue 'jvm; }
                pc = 91;
            }
            90 => {
                return;
            }
            91 => {
                local_13 = (local_11.call("getBlock", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_13, "com/simibubi/create/content/kinetics/base/IRotate"), &JvmVal::Int(0)).as_i32() == 0 { pc = 139; continue 'jvm; }
                pc = 108;
            }
            106 => {
                local_12 = (local_13).clone();
                if !jvm_ref_eq(&local_12.call("getRotationAxis", vec![local_11.clone()]), &p4.call("getAxis", vec![])) { pc = 139; continue 'jvm; }
                pc = 132;
            }
            130 => {
    let arg_tmp_0 = ctx.call_static("java/lang/Boolean", "valueOf", vec![JvmVal::Int(0)]);
                p8.call("setReturnValue", vec![arg_tmp_0.clone()]);
                pc = 139;
            }
            139 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/accessor/FallingBlockEntityAccessor.class :: create$callInit (java static)
pub fn create_call_init(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/AssertionError"));
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/accessor/FluidInteractionRegistryAccessor.class :: getInteractions (java static)
pub fn get_interactions(ctx: &mut BridgeCtx) -> JvmVal {
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/AssertionError"));
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/accessor/ItemModelGeneratorsAccessor.class :: create$getGENERATED_TRIM_MODELS (java static)
pub fn create_get_generated_trim_models(ctx: &mut BridgeCtx) -> JvmVal {
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/AssertionError"));
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/accessor/SystemReportAccessor.class :: getOPERATING_SYSTEM (java static)
pub fn get_operating_system(ctx: &mut BridgeCtx) -> JvmVal {
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/AssertionError"));
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/accessor/SystemReportAccessor.class :: getJAVA_VERSION (java static)
pub fn get_java_version(ctx: &mut BridgeCtx) -> JvmVal {
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/AssertionError"));
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/BlockPosFormatAndRenamesFixMixin.class :: create$addFixers (java instance)
pub fn create_add_fixers(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (ctx.call_static("com/simibubi/create/foundation/utility/DataFixerHelper", "BLOCK_POS_FIXERS_VIEW", vec![]).call("iterator", vec![])).clone();
                pc = 10;
            }
            9 => {
                if jvm_cmp(&local_3.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 137; continue 'jvm; }
                pc = 19;
            }
            18 => {
                local_4 = (local_3.call("next", vec![])).clone();
                local_5 = (local_4.call("reference", vec![])).clone();
                local_6 = (local_4.call("id", vec![])).clone();
                if jvm_is_null(&local_4.call("customFixer", vec![])) { pc = 110; continue 'jvm; }
                pc = 53;
            }
            51 => {
                local_8 = (ctx.call_static("com/mojang/datafixers/DSL", "namedChoice", vec![local_6.clone(), p_this.call("getInputSchema", vec![]).call("getChoiceType", vec![local_5.clone(), local_6.clone()])])).clone();
                local_7 = (p_this.call("getInputSchema", vec![]).call("getType", vec![local_5.clone()]).call("fixTypeEverywhereTyped", vec![local_8.clone(), local_4.clone(), JvmVal::Null])).clone();
                pc = 125;
            }
            110 => {
                local_7 = (p_this.call("createEntityFixer", vec![local_5.clone(), local_6.clone(), local_4.call("renames", vec![])])).clone();
                pc = 126;
            }
            125 => {
                pc = 9;
            }
            137 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/BlockPosFormatAndRenamesFixMixin.class :: lambda$create$addFixers$2 (java static)
pub fn lambda_create_add_fixers_2(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p0.call("updateTyped", vec![p1.clone(), JvmVal::Null]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/BlockPosFormatAndRenamesFixMixin.class :: lambda$create$addFixers$1 (java static)
pub fn lambda_create_add_fixers_1(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return ctx.call_static("com/mojang/datafixers/DSL", "remainderFinder", vec![]).call("update", vec![p0.clone(), JvmVal::Null]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/BlockPosFormatAndRenamesFixMixin.class :: lambda$create$addFixers$0 (java static)
pub fn lambda_create_add_fixers_0(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p0.call("customFixer", vec![]).call("apply", vec![p1.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin.class :: create$fixItemsAndTranslateNBTIntoComponents (java static)
pub fn create_fix_items_and_translate_nbt_into_components(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                p0.call("ifPresent", vec![JvmVal::Null]);
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:copper_backtank")]), &JvmVal::Int(0)).as_i32() != 0 { pc = 37; continue 'jvm; }
                pc = 29;
            }
            28 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:netherite_backtank")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 45; continue 'jvm; }
                pc = 38;
            }
            37 => {
                p0.call("moveTagToComponent", vec![JvmVal::string("Air"), JvmVal::string("create:banktank_air")]);
                pc = 46;
            }
            45 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:belt_connector")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 62; continue 'jvm; }
                pc = 55;
            }
            54 => {
                p0.call("moveTagToComponent", vec![JvmVal::string("FirstPulley"), JvmVal::string("create:belt_first_shaft")]);
                pc = 63;
            }
            62 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:handheld_worldshaper")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 143; continue 'jvm; }
                pc = 72;
            }
            71 => {
                ctx.call_static("com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin", "create$moveTagToEnumComponent", vec![p0.clone(), JvmVal::string("Pattern"), JvmVal::string("create:placement_pattern"), JvmVal::Null]);
                ctx.call_static("com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin", "create$moveTagToEnumComponent", vec![p0.clone(), JvmVal::string("Brush"), JvmVal::string("create:shaper_brush"), JvmVal::Null]);
                p0.call("moveTagToComponent", vec![JvmVal::string("BrushParams"), JvmVal::string("create:shaper_brush_params")]);
                ctx.call_static("com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin", "create$moveTagToEnumComponent", vec![p0.clone(), JvmVal::string("Placement"), JvmVal::string("create:shaper_placement_options"), JvmVal::Null]);
                ctx.call_static("com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin", "create$moveTagToEnumComponent", vec![p0.clone(), JvmVal::string("Tool"), JvmVal::string("create:shaper_tool"), JvmVal::Null]);
                p0.call("moveTagToComponent", vec![JvmVal::string("BlockUsed"), JvmVal::string("create:shaper_block_used")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("_Swap"), JvmVal::string("create:shaper_swap")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("BlockData"), JvmVal::string("create:shaper_block_data")]);
                pc = 144;
            }
            143 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:filter")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 177; continue 'jvm; }
                pc = 153;
            }
            152 => {
                ctx.call_static("com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin", "create$moveItemStackHandlerToItemContainerContents", vec![p0.clone(), p1.clone(), JvmVal::string("Items"), JvmVal::string("create:filter_items")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("RespectNBT"), JvmVal::string("create:filter_items_respect_nbt")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("Blacklist"), JvmVal::string("create:filter_items_blacklist")]);
                pc = 178;
            }
            177 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:attribute_filter")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 196; continue 'jvm; }
                pc = 187;
            }
            186 => {
                ctx.call_static("com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin", "create$moveTagToEnumComponent", vec![p0.clone(), JvmVal::string("WhitelistMode"), JvmVal::string("create:filter_items"), JvmVal::Null]);
                pc = 197;
            }
            196 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:clipboard")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 249; continue 'jvm; }
                pc = 206;
            }
            205 => {
                ctx.call_static("com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin", "create$moveTagToEnumComponent", vec![p0.clone(), JvmVal::string("Type"), JvmVal::string("create:clipboard_type"), JvmVal::Null]);
                p0.call("ifPresent", vec![JvmVal::Null]);
                p0.call("moveTagToComponent", vec![JvmVal::string("CopiedValues"), JvmVal::string("create:clipboard_copied_values")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("Pages"), JvmVal::string("create:clipboard_pages")]);
                pc = 250;
            }
            249 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:track")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 274; continue 'jvm; }
                pc = 259;
            }
            258 => {
                p0.call("moveTagToComponent", vec![JvmVal::string("ConnectingFrom"), JvmVal::string("create:track_connecting_from")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("ExtendCurve"), JvmVal::string("create:track_extended_curve")]);
                pc = 275;
            }
            274 => {
    let arg_tmp_0 = ctx.call_static("java/util/Set", "of", vec![JvmVal::string("create:track_station"), JvmVal::string("create:track_signal"), JvmVal::string("create:track_observer")]);
                if jvm_cmp(&p0.call("is", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 314; continue 'jvm; }
                pc = 291;
            }
            290 => {
                p0.call("moveTagToComponent", vec![JvmVal::string("SelectedPos"), JvmVal::string("create:track_targeting_item_selected_pos")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("SelectedDirection"), JvmVal::string("create:track_targeting_item_selected_direction")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("Bezier"), JvmVal::string("track_targeting_item_bezier")]);
                pc = 315;
            }
            314 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:schematic")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 379; continue 'jvm; }
                pc = 324;
            }
            323 => {
                p0.call("moveTagToComponent", vec![JvmVal::string("Deployed"), JvmVal::string("create:schematic_deployed")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("Owner"), JvmVal::string("create:schematic_owner")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("File"), JvmVal::string("create:schematic_file")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("Anchor"), JvmVal::string("create:schematic_anchor")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("Rotation"), JvmVal::string("create:schematic_rotation")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("Mirror"), JvmVal::string("create:schematic_mirror")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("Bounds"), JvmVal::string("create:schematic_bounds")]);
                pc = 380;
            }
            379 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:chromatic_compound")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 396; continue 'jvm; }
                pc = 389;
            }
            388 => {
                p0.call("moveTagToComponent", vec![JvmVal::string("CollectingLight"), JvmVal::string("create:chromatic_compound_collecting_light")]);
                pc = 397;
            }
            396 => {
    let arg_tmp_0 = ctx.call_static("java/util/Set", "of", vec![JvmVal::string("create:sand_paper"), JvmVal::string("create:red_sand_paper")]);
                if jvm_cmp(&p0.call("is", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 418; continue 'jvm; }
                pc = 411;
            }
            410 => {
                p0.call("moveTagToComponent", vec![JvmVal::string("Polishing"), JvmVal::string("create:sand_paper_polishing")]);
                pc = 419;
            }
            418 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:minecart_contraption")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 435; continue 'jvm; }
                pc = 428;
            }
            427 => {
                p0.call("moveTagToComponent", vec![JvmVal::string("Contraption"), JvmVal::string("create:minecart_contraption_data")]);
                pc = 436;
            }
            435 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:linked_controller")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 453; continue 'jvm; }
                pc = 445;
            }
            444 => {
                ctx.call_static("com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin", "create$moveItemStackHandlerToItemContainerContents", vec![p0.clone(), p1.clone(), JvmVal::string("Items"), JvmVal::string("create:linked_controller_items")]);
                pc = 454;
            }
            453 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin", "create$isCheckWithDyeColors", vec![p0.clone(), JvmVal::string("create:{}_toolbox")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 479; continue 'jvm; }
                pc = 463;
            }
            462 => {
                ctx.call_static("com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin", "create$moveItemStackHandlerToItemContainerContents", vec![p0.clone(), p1.clone(), JvmVal::string("Inventory"), JvmVal::string("create:toolbox_inventory")]);
                p0.call("moveTagToComponent", vec![JvmVal::string("UniqueId"), JvmVal::string("create:toolbox_uuid")]);
                pc = 480;
            }
            479 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:schedule")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 498; continue 'jvm; }
                pc = 489;
            }
            488 => {
                p0.call("moveTagToComponent", vec![JvmVal::string("Schedule"), JvmVal::string("create:train_schedule")]);
                pc = 499;
            }
            498 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:display_link")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 517; continue 'jvm; }
                pc = 509;
            }
            508 => {
                p0.call("moveTagToComponent", vec![JvmVal::string("SelectedPos"), JvmVal::string("create:display_link_selected_pos")]);
                pc = 518;
            }
            517 => {
                if jvm_cmp(&p0.call("is", vec![JvmVal::string("create:schematicannon")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 537; continue 'jvm; }
                pc = 528;
            }
            527 => {
                p0.call("moveTagToComponent", vec![JvmVal::string("BlockEntityTag.Options"), JvmVal::string("create:schematicannon_options")]);
                pc = 537;
            }
            537 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin.class :: create$isCheckWithDyeColors (java static)
pub fn create_is_check_with_dye_colors(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (JvmVal::obj("java/util/HashSet")).clone();
                local_3 = (ctx.call_static("net/minecraft/world/item/DyeColor", "values", vec![])).clone();
                local_4 = (jvm_array_length(&local_3)).clone();
                local_5 = (JvmVal::Int(0)).clone();
                pc = 21;
            }
            19 => {
                if jvm_cmp(&local_5, &local_4).as_i32() >= 0 { pc = 57; continue 'jvm; }
                pc = 27;
            }
            26 => {
                local_6 = (jvm_array_load(&local_3, local_5.as_i32())).clone();
                local_5 = jvm_iinc(&local_5, 1);
                pc = 19;
            }
            57 => {
                return p0.call("is", vec![local_2.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin.class :: create$moveTagToEnumComponent (java static)
pub fn create_move_tag_to_enum_component(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p3.call("isEnum", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 18; continue 'jvm; }
                pc = 10;
            }
            7 => {
                jvm_throw(&JvmVal::obj("java/lang/IllegalArgumentException"));
            }
            18 => {
                local_4 = (p3.call("getEnumConstants", vec![])).clone();
                p2.call("ifPresent", vec![JvmVal::Null]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin.class :: create$moveItemStackHandlerToItemContainerContents (java static)
pub fn create_move_item_stack_handler_to_item_container_contents(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                p3.call("ifPresent", vec![JvmVal::Null]);
                pc = 24;
            }
            22 => {
                local_4 = (p0).clone();
                pc = 24;
            }
            24 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin.class :: lambda$create$moveItemStackHandlerToItemContainerContents$4 (java static)
pub fn lambda_create_move_item_stack_handler_to_item_container_contents_4(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (JvmVal::obj("java/util/ArrayList")).clone();
                local_5 = (p3.call("get", vec![JvmVal::string("Items")]).call("asStream", vec![])).clone();
                local_4.call("forEach", vec![JvmVal::Null]);
                p1.call("setComponent", vec![p2.clone(), p0.call("createList", vec![local_4.call("stream", vec![])])]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin.class :: lambda$create$moveItemStackHandlerToItemContainerContents$3 (java static)
pub fn lambda_create_move_item_stack_handler_to_item_container_contents_3(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p2.call("get", vec![JvmVal::string("Slot")]).call("asInt", vec![JvmVal::Int(0)])).clone();
                local_4 = (p2.call("get", vec![JvmVal::string("id")]).call("asString", vec![]).call("getOrThrow", vec![])).clone();
                local_5 = (p2.call("get", vec![JvmVal::string("Count")]).call("asInt", vec![JvmVal::Int(1)])).clone();
                local_6 = (p0.call("emptyMap", vec![])).clone();
                local_7 = (p0.call("emptyMap", vec![])).clone();
                local_7 = (local_7.call("set", vec![JvmVal::string("count"), p0.call("createInt", vec![local_5.clone()])])).clone();
                local_7 = (local_7.call("set", vec![JvmVal::string("id"), p0.call("createString", vec![local_4.clone()])])).clone();
                local_6 = (local_6.call("set", vec![JvmVal::string("item"), local_7.clone()])).clone();
                local_6 = (local_6.call("set", vec![JvmVal::string("slot"), p0.call("createInt", vec![local_3.clone()])])).clone();
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin.class :: lambda$create$moveTagToEnumComponent$2 (java static)
pub fn lambda_create_move_tag_to_enum_component_2(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (jvm_array_load(&p0, p3.call("asInt", vec![JvmVal::Int(0)]).as_i32())).clone();
                if jvm_cmp(&jvm_instanceof(&local_4, "net/minecraft/util/StringRepresentable"), &JvmVal::Int(0)).as_i32() == 0 { pc = 36; continue 'jvm; }
                pc = 19;
            }
            17 => {
                local_6 = (local_4).clone();
                local_5 = (local_6.call("getSerializedName", vec![])).clone();
                pc = 47;
            }
            36 => {
                jvm_throw(&JvmVal::obj("java/lang/IllegalArgumentException"));
            }
            47 => {
                p1.call("setComponent", vec![p2.clone(), p3.call("emptyMap", vec![]).call("createString", vec![local_5.clone()])]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin.class :: lambda$create$fixItemsAndTranslateNBTIntoComponents$1 (java static)
pub fn lambda_create_fix_items_and_translate_nbt_into_components_1(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                p0.call("setComponent", vec![JvmVal::string("create:clipboard_read_only"), p1.call("emptyMap", vec![]).call("createString", vec![JvmVal::string("instance")])]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/ItemStackComponentizationFixMixin.class :: lambda$create$fixItemsAndTranslateNBTIntoComponents$0 (java static)
pub fn lambda_create_fix_items_and_translate_nbt_into_components_0(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p2.call("get", vec![JvmVal::string("Progress")]).call("asFloat", vec![JvmVal::Float(0_f32)])).clone();
                local_4 = (p2.call("get", vec![JvmVal::string("id")]).call("asString", vec![]).call("getOrThrow", vec![])).clone();
                local_5 = (p2.call("get", vec![JvmVal::string("Step")]).call("asInt", vec![JvmVal::Int(1)])).clone();
                local_6 = (p0.call("emptyMap", vec![])).clone();
                local_6 = (local_6.call("set", vec![JvmVal::string("progress"), p0.call("createFloat", vec![local_3.clone()])])).clone();
                local_6 = (local_6.call("set", vec![JvmVal::string("step"), p0.call("createInt", vec![local_5.clone()])])).clone();
                local_6 = (local_6.call("set", vec![JvmVal::string("id"), p0.call("createString", vec![local_4.clone()])])).clone();
                p1.call("setComponent", vec![JvmVal::string("create:sequenced_assembly"), local_6.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/V1460Mixin.class :: create$registerEntitiesToBeFixed (java instance)
pub fn create_register_entities_to_be_fixed(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p1.call("getReturnValue", vec![])).clone();
                local_4 = (ctx.call_static("com/simibubi/create/foundation/utility/DataFixerHelper", "BLOCK_POS_FIXERS_VIEW", vec![]).call("iterator", vec![])).clone();
                pc = 20;
            }
            18 => {
                if jvm_cmp(&local_4.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 64; continue 'jvm; }
                pc = 30;
            }
            28 => {
                local_5 = (local_4.call("next", vec![])).clone();
                if !jvm_ref_eq(&local_5.call("reference", vec![]), &ctx.call_static("net/minecraft/util/datafix/fixes/References", "ENTITY", vec![])) { pc = 61; continue 'jvm; }
                pc = 52;
            }
            51 => {
                p0.call("registerSimple", vec![local_3.clone(), local_5.call("id", vec![])]);
                pc = 64;
            }
            61 => {
                pc = 18;
            }
            64 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/datafixer/V1460Mixin.class :: create$registerBlockEntitiesToBeFixed (java instance)
pub fn create_register_block_entities_to_be_fixed(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p1.call("getReturnValue", vec![])).clone();
                local_4 = (ctx.call_static("com/simibubi/create/foundation/utility/DataFixerHelper", "BLOCK_POS_FIXERS_VIEW", vec![]).call("iterator", vec![])).clone();
                pc = 20;
            }
            18 => {
                if jvm_cmp(&local_4.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 64; continue 'jvm; }
                pc = 30;
            }
            28 => {
                local_5 = (local_4.call("next", vec![])).clone();
                if !jvm_ref_eq(&local_5.call("reference", vec![]), &ctx.call_static("net/minecraft/util/datafix/fixes/References", "BLOCK_ENTITY", vec![])) { pc = 61; continue 'jvm; }
                pc = 52;
            }
            51 => {
                p0.call("registerSimple", vec![local_3.clone(), local_5.call("id", vec![])]);
                pc = 64;
            }
            61 => {
                pc = 18;
            }
            64 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/accessor/HumanoidArmorLayerAccessor.class :: create$getArmorLocationCache (java static)
pub fn create_get_armor_location_cache(ctx: &mut BridgeCtx) -> JvmVal {
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/RuntimeException"));
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/client/BlockDestructionProgressMixin.class :: create$getExtraPositions (java instance)
pub fn create_get_extra_positions(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p_this.get_field("create$extraPositions");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/client/BlockDestructionProgressMixin.class :: create$setExtraPositions (java instance)
pub fn create_set_extra_positions(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                // putfield: p0 handled by the bridge object model
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/CameraMixin.class :: create$modifyCameraOffset (java instance)
pub fn create_modify_camera_offset(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return jvm_bin("*", &p0, &ctx.call_static("com/simibubi/create/content/trains/CameraDistanceModifier", "getMultiplier", vec![]));
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/client/EntityContraptionInteractionMixin.class :: create$getIntersectionContraptionsStream (java instance)
pub fn create_get_intersection_contraptions_stream(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p_this.call("filter", vec![JvmVal::Null]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/client/EntityContraptionInteractionMixin.class :: create$getIntersectingContraptions (java instance)
pub fn create_get_intersecting_contraptions(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("java/util/stream/Collectors", "toSet", vec![]);
                local_1 = (p_this.call("create$getIntersectionContraptionsStream", vec![]).call("collect", vec![arg_tmp_0.clone()])).clone();
                return local_1;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/client/EntityContraptionInteractionMixin.class :: create$forCollision (java instance)
pub fn create_for_collision(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                p1.call("forEach", vec![JvmVal::Null]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/EntityContraptionInteractionMixin.class :: create$contraptionStepSounds (java instance)
pub fn create_contraption_step_sounds(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (p_this.get_field("position").call("add", vec![JvmVal::Double(0_f64), JvmVal::Double(-0.2_f64), JvmVal::Double(0_f64)])).clone();
                local_5 = (JvmVal::obj("org/apache/commons/lang3/mutable/MutableBoolean")).clone();
                p_this.call("create$forCollision", vec![local_5.clone(), JvmVal::Null]);
                if jvm_cmp(&local_5.call("booleanValue", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 54; continue 'jvm; }
                pc = 47;
            }
            46 => {
                // putfield: p_this.call("nextStep", vec![]) handled by the bridge object model
                pc = 54;
            }
            54 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/EntityContraptionInteractionMixin.class :: create$onMove (java instance)
pub fn create_on_move(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p_this.get_field("level").get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 11; continue 'jvm; }
                pc = 11;
            }
            10 => {
                return;
            }
            11 => {
                local_4 = (p_this).clone();
                if jvm_cmp(&local_4.call("onGround", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 26; continue 'jvm; }
                pc = 26;
            }
            25 => {
                return;
            }
            26 => {
                if jvm_cmp(&local_4.call("isPassenger", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 35; continue 'jvm; }
                pc = 35;
            }
            34 => {
                return;
            }
            35 => {
                local_5 = (p_this.get_field("position").call("add", vec![JvmVal::Double(0_f64), JvmVal::Double(-0.2_f64), JvmVal::Double(0_f64)])).clone();
                local_6 = (local_4.call("anyMatch", vec![JvmVal::Null])).clone();
                if jvm_cmp(&local_6, &JvmVal::Int(0)).as_i32() != 0 { pc = 75; continue 'jvm; }
                pc = 75;
            }
            74 => {
                return;
            }
            75 => {
                local_4.call("setOnGround", vec![JvmVal::Int(1)]);
                local_4.call("getPersistentData", vec![]).call("putBoolean", vec![JvmVal::string("ContraptionGrounded"), JvmVal::Int(1)]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/EntityContraptionInteractionMixin.class :: create$onSpawnSprintParticle (java instance)
pub fn create_on_spawn_sprint_particle(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (p_this).clone();
                local_3 = (p_this.get_field("position").call("add", vec![JvmVal::Double(0_f64), JvmVal::Double(-0.2_f64), JvmVal::Double(0_f64)])).clone();
                local_4 = (ctx.call_static("net/minecraft/core/BlockPos", "containing", vec![local_3.clone()])).clone();
                local_2.call("create$forCollision", vec![local_4.clone(), JvmVal::Null]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/EntityContraptionInteractionMixin.class :: lambda$create$onSpawnSprintParticle$4 (java instance)
pub fn lambda_create_on_spawn_sprint_particle_4(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p3.call("addRunningEffects", vec![p_this.get_field("level"), p4.clone(), p0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 140; continue 'jvm; }
                pc = 17;
            }
            15 => {
                if jvm_ref_eq(&p3.call("getRenderShape", vec![]), &ctx.call_static("net/minecraft/world/level/block/RenderShape", "INVISIBLE", vec![])) { pc = 140; continue 'jvm; }
                pc = 27;
            }
            26 => {
                local_6 = (p0.call("getDeltaMovement", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/particles/ParticleTypes", "BLOCK", vec![]);
                p_this.get_field("level").call("addParticle", vec![JvmVal::obj("net/minecraft/core/particles/BlockParticleOption").call("setPos", vec![p1.clone()]), jvm_bin("+", &p0.call("getX", vec![]), &jvm_bin("*", &jvm_bin("-", &jvm_conv(&p_this.get_field("random").call("nextFloat", vec![]), "D"), &JvmVal::Double(0.5_f64)), &jvm_conv(&p_this.get_field("dimensions").call("width", vec![]), "D"))), jvm_bin("+", &p0.call("getY", vec![]), &JvmVal::Double(0.1_f64)), jvm_bin("+", &p0.call("getZ", vec![]), &jvm_bin("*", &jvm_bin("-", &jvm_conv(&p_this.get_field("random").call("nextFloat", vec![]), "D"), &JvmVal::Double(0.5_f64)), &jvm_conv(&p_this.get_field("dimensions").call("height", vec![]), "D"))), jvm_bin("*", &local_6.get_field("x"), &JvmVal::Double(-4_f64)), JvmVal::Double(1.5_f64), jvm_bin("*", &local_6.get_field("z"), &JvmVal::Double(-4_f64))]);
                pc = 140;
            }
            140 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/EntityContraptionInteractionMixin.class :: lambda$create$onMove$3 (java static)
pub fn lambda_create_on_move_3(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (ctx.call_static("com/simibubi/create/content/contraptions/ContraptionCollider", "worldToLocalPos", vec![p0.clone(), p2.clone()])).clone();
                local_4 = (ctx.call_static("net/minecraft/core/BlockPos", "containing", vec![local_3.clone()])).clone();
                local_5 = (p2.call("getContraption", vec![])).clone();
                local_6 = (local_5.call("getBlocks", vec![]).call("get", vec![local_4.clone()])).clone();
                if !jvm_is_null(&local_6) { pc = 42; continue 'jvm; }
                pc = 41;
            }
            40 => {
                return JvmVal::Int(0);
            }
            42 => {
                p2.call("registerColliding", vec![p1.clone()]);
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/client/EntityContraptionInteractionMixin.class :: lambda$create$contraptionStepSounds$2 (java instance)
pub fn lambda_create_contraption_step_sounds_2(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                p_this.call("playStepSound", vec![p3.clone(), p2.clone()]);
                p0.call("setTrue", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/EntityContraptionInteractionMixin.class :: lambda$create$forCollision$1 (java static)
pub fn lambda_create_for_collision_1(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (ctx.call_static("com/simibubi/create/content/contraptions/ContraptionCollider", "worldToLocalPos", vec![p0.clone(), p2.clone()])).clone();
                local_4 = (ctx.call_static("net/minecraft/core/BlockPos", "containing", vec![local_3.clone()])).clone();
                local_5 = (p2.call("getContraption", vec![])).clone();
                local_6 = (local_5.call("getBlocks", vec![]).call("get", vec![local_4.clone()])).clone();
                if jvm_is_null(&local_6) { pc = 59; continue 'jvm; }
                pc = 42;
            }
            40 => {
                local_7 = (local_6.call("state", vec![])).clone();
                p1.call("accept", vec![local_5.clone(), local_7.clone(), local_4.clone()]);
                pc = 59;
            }
            59 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/EntityContraptionInteractionMixin.class :: lambda$create$getIntersectionContraptionsStream$0 (java instance)
pub fn lambda_create_get_intersection_contraptions_stream_0(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_is_null(&p0) { pc = 24; continue 'jvm; }
                pc = 5;
            }
            4 => {
                if jvm_cmp(&p0.get_field("collidingEntities").call("containsKey", vec![p_this.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 24; continue 'jvm; }
                pc = 21;
            }
            20 => {
                pc = 25;
            }
            24 => {
                pc = 25;
            }
            25 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/client/GameRendererMixin.class :: create$bigShapePick (java instance)
pub fn create_big_shape_pick(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/foundation/block/BigOutlines", "pick", vec![]);
                ctx.call_static("com/simibubi/create/content/trains/track/TrackBlockOutline", "pickCurves", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/HeavyBootsOnPlayerMixin.class :: create$noSwimmingWithHeavyBootsOn (java instance)
pub fn create_no_swimming_with_heavy_boots_on(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (p_this.call("getPersistentData", vec![])).clone();
                if jvm_cmp(&local_2.call("contains", vec![JvmVal::string("HeavyBoots")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 22; continue 'jvm; }
                pc = 15;
            }
            14 => {
    let arg_tmp_0 = ctx.call_static("java/lang/Boolean", "valueOf", vec![JvmVal::Int(0)]);
                p0.call("setReturnValue", vec![arg_tmp_0.clone()]);
                pc = 22;
            }
            22 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/HumanoidArmorLayerMixin.class :: create$onRenderArmorPiece (java instance)
pub fn create_on_render_armor_piece(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal, p5: JvmVal, p6: JvmVal, p7: JvmVal, p8: JvmVal, p9: JvmVal, p10: JvmVal, p11: JvmVal, p12: JvmVal, p13: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut p5: JvmVal = p5.clone();
    let mut p6: JvmVal = p6.clone();
    let mut p7: JvmVal = p7.clone();
    let mut p8: JvmVal = p8.clone();
    let mut p9: JvmVal = p9.clone();
    let mut p10: JvmVal = p10.clone();
    let mut p11: JvmVal = p11.clone();
    let mut p12: JvmVal = p12.clone();
    let mut p13: JvmVal = p13.clone();
    let mut local_15: JvmVal = JvmVal::Null;
    let mut local_16: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_16 = (p13.call("getItem", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_16, "com/simibubi/create/foundation/item/CustomRenderedArmorItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 49; continue 'jvm; }
                pc = 17;
            }
            15 => {
                local_15 = (local_16).clone();
                local_15.call("renderArmorPiece", vec![p_this.clone(), p0.clone(), p1.clone(), p2.clone(), p3.clone(), p4.clone(), p5.clone(), p13.clone()]);
                p12.call("cancel", vec![]);
                pc = 49;
            }
            49 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/HumanoidModelMixin.class :: create$afterSetupAnim (java instance)
pub fn create_after_setup_anim(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal, p5: JvmVal, p6: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut p5: JvmVal = p5.clone();
    let mut p6: JvmVal = p6.clone();
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p0, "net/minecraft/client/player/AbstractClientPlayer"), &JvmVal::Int(0)).as_i32() == 0 { pc = 16; continue 'jvm; }
                pc = 8;
            }
            7 => {
                local_8 = (p0).clone();
                pc = 17;
            }
            16 => {
                return;
            }
            17 => {
                ctx.call_static("com/simibubi/create/foundation/render/PlayerSkyhookRenderer", "afterSetupAnim", vec![local_8.clone(), p_this.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/HumanoidModelMixin.class :: create$beforeSetupAnim (java instance)
pub fn create_before_setup_anim(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal, p5: JvmVal, p6: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut p5: JvmVal = p5.clone();
    let mut p6: JvmVal = p6.clone();
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p0, "net/minecraft/client/player/AbstractClientPlayer"), &JvmVal::Int(0)).as_i32() == 0 { pc = 16; continue 'jvm; }
                pc = 8;
            }
            7 => {
                local_8 = (p0).clone();
                pc = 17;
            }
            16 => {
                return;
            }
            17 => {
                ctx.call_static("com/simibubi/create/foundation/render/PlayerSkyhookRenderer", "beforeSetupAnim", vec![local_8.clone(), p_this.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/LevelRendererMixin.class :: create$onDestroyBlockProgress (java instance)
pub fn create_on_destroy_block_progress(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_6 = (p_this.get_field("level").call("getBlockState", vec![p1.clone()])).clone();
                local_7 = (ctx.call_static("net/neoforged/neoforge/client/extensions/common/IClientBlockExtensions", "of", vec![local_6.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_7, "com/simibubi/create/foundation/block/render/MultiPosDestructionHandler"), &JvmVal::Int(0)).as_i32() == 0 { pc = 139; continue 'jvm; }
                pc = 27;
            }
            25 => {
                local_8 = (local_7).clone();
                local_9 = (local_8.call("getExtraPositions", vec![p_this.get_field("level"), p1.clone(), local_6.clone(), p2.clone()])).clone();
                if jvm_is_null(&local_9) { pc = 139; continue 'jvm; }
                pc = 56;
            }
            54 => {
                p4.call("create$setExtraPositions", vec![local_9.clone()]);
                local_10 = (local_9.call("iterator", vec![])).clone();
                pc = 86;
            }
            84 => {
                if jvm_cmp(&local_10.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 139; continue 'jvm; }
                pc = 96;
            }
            94 => {
                local_11 = (local_10.call("next", vec![])).clone();
                pc = 84;
            }
            139 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/LevelRendererMixin.class :: create$onRemoveProgress (java instance)
pub fn create_on_remove_progress(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p0.call("create$getExtraPositions", vec![])).clone();
                if jvm_is_null(&local_3) { pc = 106; continue 'jvm; }
                pc = 15;
            }
            14 => {
                local_4 = (local_3.call("iterator", vec![])).clone();
                pc = 24;
            }
            22 => {
                if jvm_cmp(&local_4.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 106; continue 'jvm; }
                pc = 34;
            }
            32 => {
                local_5 = (local_4.call("next", vec![])).clone();
                local_6 = (local_5.call("asLong", vec![])).clone();
                local_8 = (p_this.get_field("destructionProgress").call("get", vec![local_6.clone()])).clone();
                if jvm_is_null(&local_8) { pc = 103; continue 'jvm; }
                pc = 74;
            }
            72 => {
                if jvm_cmp(&local_8.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 103; continue 'jvm; }
                pc = 92;
            }
            91 => {
                pc = 106;
            }
            103 => {
                pc = 22;
            }
            106 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/client/LevelRendererMixin.class :: lambda$create$onDestroyBlockProgress$0 (java static)
pub fn lambda_create_on_destroy_block_progress_0(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return ctx.call_static("com/google/common/collect/Sets", "newTreeSet", vec![]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/mixin/client/PlayerRendererMixin.class :: create$onGetArmPose (java static)
pub fn create_on_get_arm_pose(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p0.call("getItemInHand", vec![p1.clone()])).clone();
                local_5 = (local_3.call("getItem", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_5, "com/simibubi/create/foundation/item/CustomArmPoseItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 50; continue 'jvm; }
                pc = 22;
            }
            20 => {
                local_4 = (local_5).clone();
                local_5 = (local_4.call("getArmPose", vec![local_3.clone(), p0.clone(), p1.clone()])).clone();
                if jvm_is_null(&local_5) { pc = 50; continue 'jvm; }
                pc = 45;
            }
            44 => {
                p2.call("setReturnValue", vec![local_5.clone()]);
                pc = 50;
            }
            50 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/compat/xaeros/XaeroFullscreenMapMixin.class :: create$xaeroMapFullscreenRender (java instance)
pub fn create_xaero_map_fullscreen_render(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p_this.get_field("create$failedToRenderTrainMap"), &JvmVal::Int(0)).as_i32() != 0 { pc = 19; continue 'jvm; }
                pc = 8;
            }
            7 => {
                ctx.call_static("com/simibubi/create/compat/trainmap/XaeroTrainMap", "onRender", vec![p0.clone(), p_this.clone(), p1.clone(), p2.clone(), p3.clone()]);
                pc = 22;
            }
            19 => {
                pc = 41;
            }
            22 => {
                local_6 = (JvmVal::Null).clone();
                ctx.call_static("com/simibubi/create/Create", "LOGGER", vec![]).call("error", vec![JvmVal::string("Failed to render Xaero's World Map train map integration:"), local_6.clone()]);
                // putfield: JvmVal::Int(1) handled by the bridge object model
                pc = 41;
            }
            41 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/mixin/compat/xaeros/XaeroPauseScreenOverrideMixin.class :: create$xaeroPauseScreenOverride (java instance)
pub fn create_xaero_pause_screen_override(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/compat/Mods", "XAEROWORLDMAP", vec![]).call("isLoaded", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 27; continue 'jvm; }
                pc = 10;
            }
            9 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/compat/trainmap/XaeroTrainMap", "isMapOpen", vec![p_this.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 27; continue 'jvm; }
                pc = 20;
            }
            19 => {
    let arg_tmp_0 = ctx.call_static("java/lang/Boolean", "valueOf", vec![JvmVal::Int(0)]);
                p0.call("setReturnValue", vec![arg_tmp_0.clone()]);
                pc = 27;
            }
            27 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/GlStateManagerMixin.class :: flywheel$onBindBuffer (java static)
pub fn flywheel_on_bind_buffer(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("dev/engine_room/flywheel/backend/gl/buffer/GlBufferType", "fromTarget", vec![p0.clone()]);
                ctx.call_static("dev/engine_room/flywheel/backend/gl/GlStateTracker", "_setBuffer", vec![arg_tmp_0.clone(), p1.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/GlStateManagerMixin.class :: flywheel$onBindVertexArray (java static)
pub fn flywheel_on_bind_vertex_array(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("dev/engine_room/flywheel/backend/gl/GlStateTracker", "_setVertexArray", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/GlStateManagerMixin.class :: flywheel$onUseProgram (java static)
pub fn flywheel_on_use_program(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("dev/engine_room/flywheel/backend/gl/GlStateTracker", "_setProgram", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/GlStateManagerMixin.class :: flywheel$onSetupLevelDiffuseLighting (java static)
pub fn flywheel_on_setup_level_diffuse_lighting(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/OptionsMixin.class :: flywheel$onLoad (java instance)
pub fn flywheel_on_load(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("dev/engine_room/flywheel/backend/engine/uniform/OptionsUniforms", "update", vec![p_this.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/OptionsMixin.class :: flywheel$onSave (java instance)
pub fn flywheel_on_save(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("dev/engine_room/flywheel/backend/engine/uniform/OptionsUniforms", "update", vec![p_this.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/RenderSystemMixin.class :: flywheel$onInitRenderer (java static)
pub fn flywheel_on_init_renderer(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("dev/engine_room/flywheel/backend/gl/GlCompat", "init", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/RenderSystemMixin.class :: flywheel$onSetFogStart (java static)
pub fn flywheel_on_set_fog_start(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("dev/engine_room/flywheel/backend/engine/uniform/FogUniforms", "update", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/RenderSystemMixin.class :: flywheel$onSetFogEnd (java static)
pub fn flywheel_on_set_fog_end(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("dev/engine_room/flywheel/backend/engine/uniform/FogUniforms", "update", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/RenderSystemMixin.class :: flywheel$onSetFogShape (java static)
pub fn flywheel_on_set_fog_shape(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("dev/engine_room/flywheel/backend/engine/uniform/FogUniforms", "update", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/RenderSystemMixin.class :: flywheel$onSetFogColor (java static)
pub fn flywheel_on_set_fog_color(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("dev/engine_room/flywheel/backend/engine/uniform/FogUniforms", "update", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/backend/mixin/light/SkyLightSectionStorageMixin.class :: flywheel$skyDataLayer (java instance)
pub fn flywheel_sky_data_layer(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p0).clone();
                local_5 = (ctx.call_static("net/minecraft/core/SectionPos", "y", vec![local_3.clone()])).clone();
                local_6 = (p_this.get_field("visibleSectionData")).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/SectionPos", "getZeroNode", vec![local_3.clone()]);
                local_7 = (local_6.call("flywheel$topSections", vec![]).call("get", vec![arg_tmp_0.clone()])).clone();
                if jvm_cmp(&local_7, &local_6.call("flywheel$currentLowestY", vec![])).as_i32() == 0 { pc = 102; continue 'jvm; }
                pc = 47;
            }
            45 => {
                if jvm_cmp(&local_5, &local_7).as_i32() >= 0 { pc = 102; continue 'jvm; }
                pc = 53;
            }
            52 => {
                local_8 = (p_this.call("getDataLayerData", vec![local_3.clone()])).clone();
                if !jvm_is_null(&local_8) { pc = 99; continue 'jvm; }
                pc = 66;
            }
            64 => {
                if !jvm_is_null(&local_8) { pc = 99; continue 'jvm; }
                pc = 72;
            }
            69 => {
                local_5 = jvm_iinc(&local_5, 1);
                if jvm_cmp(&local_5, &local_7).as_i32() < 0 { pc = 81; continue 'jvm; }
                pc = 80;
            }
            79 => {
                return JvmVal::Null;
            }
            81 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/Direction", "UP", vec![]);
                local_3 = (ctx.call_static("net/minecraft/core/SectionPos", "offset", vec![local_3.clone(), arg_tmp_0.clone()])).clone();
                local_8 = (p_this.call("getDataLayerData", vec![local_3.clone()])).clone();
                pc = 64;
            }
            99 => {
                return local_8;
            }
            102 => {
                return JvmVal::Null;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/BlockEntityTypeMixin.class :: flywheel$getVisualizer (java instance)
pub fn flywheel_get_visualizer(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p_this.get_field("flywheel$visualizer");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/BlockEntityTypeMixin.class :: flywheel$setVisualizer (java instance)
pub fn flywheel_set_visualizer(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&ctx.call_static("dev/engine_room/flywheel/impl/compat/SodiumCompat", "ACTIVE", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 26; continue 'jvm; }
                pc = 7;
            }
            6 => {
                // putfield: ctx.call_static("dev/engine_room/flywheel/impl/compat/SodiumCompat", "onSetBlockEntityVisualizer", vec![p_this.clone(), p_this.get_field("flywheel$visualizer"), p0.clone(), p_this.get_field("flywheel$sodiumPredicate")]) handled by the bridge object model
                pc = 27;
            }
            26 => {
                // putfield: p0 handled by the bridge object model
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/ClientChunkCacheMixin.class :: flywheel$onLightUpdate (java instance)
pub fn flywheel_on_light_update(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (ctx.call_static("dev/engine_room/flywheel/impl/visualization/VisualizationManagerImpl", "get", vec![p_this.get_field("level")])).clone();
                if jvm_is_null(&local_4) { pc = 21; continue 'jvm; }
                pc = 16;
            }
            14 => {
                local_4.call("onLightUpdate", vec![p1.clone(), p0.clone()]);
                pc = 21;
            }
            21 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/EntityTypeMixin.class :: flywheel$getVisualizer (java instance)
pub fn flywheel_get_visualizer_2(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p_this.get_field("flywheel$visualizer");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/EntityTypeMixin.class :: flywheel$setVisualizer (java instance)
pub fn flywheel_set_visualizer_2(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                // putfield: p0 handled by the bridge object model
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/LevelMixin.class :: flywheel$getAllLoadedEntities (java instance)
pub fn flywheel_get_all_loaded_entities(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p_this.call("getEntities", vec![]).call("getAll", vec![]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/LevelRendererMixin.class :: flywheel$beginRender (java instance)
pub fn flywheel_begin_render(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal, p5: JvmVal, p6: JvmVal, p7: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut p5: JvmVal = p5.clone();
    let mut p6: JvmVal = p6.clone();
    let mut p7: JvmVal = p7.clone();
    let mut local_9: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                // putfield: ctx.call_static("dev/engine_room/flywheel/impl/event/RenderContextImpl", "create", vec![p_this.clone(), p_this.get_field("level"), p_this.get_field("renderBuffers"), p5.clone(), p6.clone(), p2.clone(), p0.call("getGameTimeDeltaPartialTick", vec![JvmVal::Int(0)])]) handled by the bridge object model
                local_9 = (ctx.call_static("dev/engine_room/flywheel/api/visualization/VisualizationManager", "get", vec![p_this.get_field("level")])).clone();
                if jvm_is_null(&local_9) { pc = 61; continue 'jvm; }
                pc = 47;
            }
            45 => {
                local_9.call("renderDispatcher", vec![]).call("onStartLevelRender", vec![p_this.get_field("flywheel$renderContext")]);
                pc = 61;
            }
            61 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/LevelRendererMixin.class :: flywheel$endRender (java instance)
pub fn flywheel_end_render(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                // putfield: JvmVal::Null handled by the bridge object model
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/LevelRendererMixin.class :: flywheel$reload (java instance)
pub fn flywheel_reload(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_is_null(&p_this.get_field("level")) { pc = 19; continue 'jvm; }
                pc = 10;
            }
            7 => {
                ctx.call_static("dev/engine_room/flywheel/impl/FlwImplXplat", "INSTANCE", vec![]).call("dispatchReloadLevelRendererEvent", vec![p_this.get_field("level")]);
                pc = 19;
            }
            19 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/LevelRendererMixin.class :: flywheel$beforeBlockEntities (java instance)
pub fn flywheel_before_block_entities(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_is_null(&p_this.get_field("flywheel$renderContext")) { pc = 34; continue 'jvm; }
                pc = 8;
            }
            7 => {
                local_2 = (ctx.call_static("dev/engine_room/flywheel/api/visualization/VisualizationManager", "get", vec![p_this.get_field("level")])).clone();
                if jvm_is_null(&local_2) { pc = 34; continue 'jvm; }
                pc = 20;
            }
            19 => {
                local_2.call("renderDispatcher", vec![]).call("afterEntities", vec![p_this.get_field("flywheel$renderContext")]);
                pc = 34;
            }
            34 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/LevelRendererMixin.class :: flywheel$beforeRenderCrumbling (java instance)
pub fn flywheel_before_render_crumbling(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_is_null(&p_this.get_field("flywheel$renderContext")) { pc = 38; continue 'jvm; }
                pc = 8;
            }
            7 => {
                local_2 = (ctx.call_static("dev/engine_room/flywheel/api/visualization/VisualizationManager", "get", vec![p_this.get_field("level")])).clone();
                if jvm_is_null(&local_2) { pc = 38; continue 'jvm; }
                pc = 20;
            }
            19 => {
                local_2.call("renderDispatcher", vec![]).call("beforeCrumbling", vec![p_this.get_field("flywheel$renderContext"), p_this.get_field("destructionProgress")]);
                pc = 38;
            }
            38 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/LevelRendererMixin.class :: flywheel$decideNotToRenderEntity (java instance)
pub fn flywheel_decide_not_to_render_entity(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal, p5: JvmVal, p6: JvmVal, p7: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut p5: JvmVal = p5.clone();
    let mut p6: JvmVal = p6.clone();
    let mut p7: JvmVal = p7.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&ctx.call_static("dev/engine_room/flywheel/api/visualization/VisualizationManager", "supportsVisualization", vec![p0.call("level", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 22; continue 'jvm; }
                pc = 11;
            }
            10 => {
                if jvm_cmp(&ctx.call_static("dev/engine_room/flywheel/lib/visualization/VisualizationHelper", "skipVanillaRender", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 22; continue 'jvm; }
                pc = 19;
            }
            17 => {
                p7.call("cancel", vec![]);
                pc = 22;
            }
            22 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/MinecraftMixin.class :: flywheel$onBeginInitialResourceReload (java instance)
pub fn flywheel_on_begin_initial_resource_reload(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("dev/engine_room/flywheel/impl/FlwImpl", "freezeRegistries", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/MinecraftMixin.class :: flywheel$onEndInitialResourceReload (java instance)
pub fn flywheel_on_end_initial_resource_reload(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("net/neoforged/fml/ModLoader", "postEvent", vec![JvmVal::obj("dev/engine_room/flywheel/api/event/EndClientResourceReloadEvent")]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/MinecraftMixin.class :: flywheel$onEndManualResourceReload (java instance)
pub fn flywheel_on_end_manual_resource_reload(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("net/neoforged/fml/ModLoader", "postEvent", vec![JvmVal::obj("dev/engine_room/flywheel/api/event/EndClientResourceReloadEvent")]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/PoseStackMixin.class :: flywheel$transformStack (java instance)
pub fn flywheel_transform_stack(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_is_null(&p_this.get_field("flywheel$wrapper")) { pc = 22; continue 'jvm; }
                pc = 8;
            }
            7 => {
                // putfield: JvmVal::obj("dev/engine_room/flywheel/lib/transform/PoseTransformStack") handled by the bridge object model
                pc = 23;
            }
            22 => {
                return p_this.get_field("flywheel$wrapper");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/fix/FixFabulousDepthMixin.class :: flywheel$disableTransparencyShaderDepth (java instance)
pub fn flywheel_disable_transparency_shader_depth(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/mojang/blaze3d/platform/GlStateManager", "_depthMask", vec![JvmVal::Int(0)]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/visualmanage/BlockEntityMixin.class :: flywheel$removeVisual (java instance)
pub fn flywheel_remove_visual(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (ctx.call_static("dev/engine_room/flywheel/api/visualization/VisualizationManager", "get", vec![p_this.get_field("level")])).clone();
                if !jvm_is_null(&local_2) { pc = 13; continue 'jvm; }
                pc = 13;
            }
            12 => {
                return;
            }
            13 => {
                local_2.call("blockEntities", vec![]).call("queueRemove", vec![p_this.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/visualmanage/LevelChunkMixin.class :: flywheel$onBlockEntityAdded (java instance)
pub fn flywheel_on_block_entity_added(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (ctx.call_static("dev/engine_room/flywheel/api/visualization/VisualizationManager", "get", vec![p_this.get_field("level")])).clone();
                if !jvm_is_null(&local_3) { pc = 13; continue 'jvm; }
                pc = 13;
            }
            12 => {
                return;
            }
            13 => {
                local_3.call("blockEntities", vec![]).call("queueAdd", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/visualmanage/LevelRendererMixin.class :: flywheel$checkUpdate (java instance)
pub fn flywheel_check_update(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_5 = (ctx.call_static("dev/engine_room/flywheel/api/visualization/VisualizationManager", "get", vec![p_this.get_field("level")])).clone();
                if !jvm_is_null(&local_5) { pc = 15; continue 'jvm; }
                pc = 15;
            }
            14 => {
                return;
            }
            15 => {
                local_6 = (p_this.get_field("level").call("getBlockEntity", vec![p0.clone()])).clone();
                if !jvm_is_null(&local_6) { pc = 31; continue 'jvm; }
                pc = 31;
            }
            30 => {
                return;
            }
            31 => {
                local_7 = (local_5.call("blockEntities", vec![])).clone();
                if jvm_cmp(&p1, &p2).as_i32() == 0 { pc = 66; continue 'jvm; }
                pc = 47;
            }
            45 => {
                local_7.call("queueRemove", vec![local_6.clone()]);
                local_7.call("queueAdd", vec![local_6.clone()]);
                pc = 75;
            }
            66 => {
                local_7.call("queueUpdate", vec![local_6.clone()]);
                pc = 75;
            }
            75 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/visualmanage/SectionCompilerMixin.class :: flywheel$tryAddBlockEntity (java instance)
pub fn flywheel_try_add_block_entity(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&ctx.call_static("dev/engine_room/flywheel/lib/visualization/VisualizationHelper", "tryAddBlockEntity", vec![p1.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 11; continue 'jvm; }
                pc = 8;
            }
            7 => {
                p2.call("cancel", vec![]);
                pc = 11;
            }
            11 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/flywheel-neoforge-1.21.1-1.0.6!dev/engine_room/flywheel/impl/mixin/neoforge/ModelBlockRendererMixin.class :: onTesselateBlock (java instance)
pub fn on_tesselate_block(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal, p5: JvmVal, p6: JvmVal, p7: JvmVal, p8: JvmVal, p9: JvmVal, p10: JvmVal, p11: JvmVal, p12: JvmVal, p13: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut p5: JvmVal = p5.clone();
    let mut p6: JvmVal = p6.clone();
    let mut p7: JvmVal = p7.clone();
    let mut p8: JvmVal = p8.clone();
    let mut p9: JvmVal = p9.clone();
    let mut p10: JvmVal = p10.clone();
    let mut p11: JvmVal = p11.clone();
    let mut p12: JvmVal = p12.clone();
    let mut p13: JvmVal = p13.clone();
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_16: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p5, "dev/engine_room/flywheel/lib/model/baked/NeoforgeMeshEmitter"), &JvmVal::Int(0)).as_i32() == 0 { pc = 22; continue 'jvm; }
                pc = 10;
            }
            8 => {
                local_16 = (p5).clone();
                local_16.call("prepareForModelLayer", vec![p13.clone()]);
                pc = 22;
            }
            22 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/ponder-neoforge-1.0.82+mc1.21.1!net/createmod/ponder/mixin/client/WindowResizeMixin.class :: catnip$updateWindowSize (java instance)
pub fn catnip_update_window_size(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("net/createmod/catnip/gui/UIRenderHelper", "updateWindowSize", vec![p_this.get_field("window")]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from META-INF/jarjar/ponder-neoforge-1.0.82+mc1.21.1!net/createmod/ponder/mixin/client/accessor/RenderSystemAccessor.class :: catnip$getShaderLightDirections (java static)
pub fn catnip_get_shader_light_directions(ctx: &mut BridgeCtx) -> JvmVal {
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/AssertionError"));
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from META-INF/jarjar/ponder-neoforge-1.0.82+mc1.21.1!net/createmod/ponder/mixin/client/accessor/RenderTypeAccessor.class :: catnip$create (java static)
pub fn catnip_create(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal, p5: JvmVal, p6: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut p5: JvmVal = p5.clone();
    let mut p6: JvmVal = p6.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                jvm_throw(&JvmVal::obj("java/lang/AssertionError"));
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler.class :: rightClickingBlocksSelectsThem (java static)
pub fn right_clicking_blocks_selects_them(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentItem", vec![])) { pc = 7; continue 'jvm; }
                pc = 7;
            }
            6 => {
                return;
            }
            7 => {
                local_1 = (p0.call("getPos", vec![])).clone();
                local_2 = (p0.call("getLevel", vec![])).clone();
                if jvm_cmp(&local_2.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 25; continue 'jvm; }
                pc = 25;
            }
            24 => {
                return;
            }
            25 => {
                local_3 = (p0.call("getEntity", vec![])).clone();
                if jvm_is_null(&local_3) { pc = 42; continue 'jvm; }
                pc = 35;
            }
            34 => {
                if jvm_cmp(&local_3.call("isSpectator", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 42; continue 'jvm; }
                pc = 42;
            }
            41 => {
                return;
            }
            42 => {
                local_4 = (ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "getSelected", vec![local_1.clone()])).clone();
                local_5 = (local_2.call("getBlockState", vec![local_1.clone()])).clone();
                if !jvm_is_null(&local_4) { pc = 84; continue 'jvm; }
                pc = 61;
            }
            60 => {
                local_6 = (ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPoint", "create", vec![local_2.clone(), local_1.clone(), local_5.clone()])).clone();
                if !jvm_is_null(&local_6) { pc = 75; continue 'jvm; }
                pc = 75;
            }
            74 => {
                return;
            }
            75 => {
                local_4 = (local_6).clone();
                ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "put", vec![local_6.clone()]);
                pc = 86;
            }
            84 => {
                local_4.call("cycleMode", vec![]);
                if jvm_is_null(&local_3) { pc = 141; continue 'jvm; }
                pc = 95;
            }
            93 => {
                local_6 = (local_4.call("getMode", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "WHITE", vec![]);
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "blockName", vec![local_5.clone()]).call("style", vec![arg_tmp_0.clone()]));
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "builder", vec![]).call("translate", vec![local_6.call("getTranslationKey", vec![]), jvm_array_new(0)]).call("color", vec![local_6.call("getColor", vec![])]).call("sendStatus", vec![local_3.clone()]);
                pc = 142;
            }
            141 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler.class :: leftClickingBlocksDeselectsThem (java static)
pub fn left_clicking_blocks_deselects_them(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentItem", vec![])) { pc = 7; continue 'jvm; }
                pc = 7;
            }
            6 => {
                return;
            }
            7 => {
                if jvm_cmp(&p0.call("getLevel", vec![]).get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 18; continue 'jvm; }
                pc = 18;
            }
            17 => {
                return;
            }
            18 => {
                local_1 = (p0.call("getPos", vec![])).clone();
                if jvm_is_null(&ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "remove", vec![local_1.clone()])) { pc = 35; continue 'jvm; }
                pc = 31;
            }
            30 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                pc = 35;
            }
            35 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler.class :: flushSettings (java static)
pub fn flush_settings(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentSelection", vec![])) { pc = 7; continue 'jvm; }
                pc = 7;
            }
            6 => {
                return;
            }
            7 => {
                local_1 = (JvmVal::Int(0)).clone();
                local_2 = (ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentSelection", vec![]).call("iterator", vec![])).clone();
                pc = 19;
            }
            18 => {
                if jvm_cmp(&local_2.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 67; continue 'jvm; }
                pc = 28;
            }
            27 => {
                local_3 = (local_2.call("next", vec![])).clone();
    let arg_tmp_0 = jvm_conv(&ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmBlockEntity", "getRange", vec![]), "D");
                if jvm_cmp(&local_3.call("getPos", vec![]).call("closerThan", vec![p0.clone(), arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 55; continue 'jvm; }
                pc = 55;
            }
            52 => {
                pc = 18;
            }
            55 => {
                local_2.call("remove", vec![]);
                local_1 = jvm_iinc(&local_1, 1);
                pc = 18;
            }
            67 => {
                local_2 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("player")).clone();
                if jvm_cmp(&local_1, &JvmVal::Int(0)).as_i32() <= 0 { pc = 110; continue 'jvm; }
                pc = 81;
            }
            78 => {
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), ctx.call_static("java/lang/Integer", "valueOf", vec![local_1.clone()]));
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "builder", vec![]).call("translate", vec![JvmVal::string("mechanical_arm.points_outside_range"), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("sendStatus", vec![local_2.clone()]);
                pc = 214;
            }
            110 => {
                local_3 = (JvmVal::Int(0)).clone();
                local_4 = (JvmVal::Int(0)).clone();
                local_5 = (ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentSelection", vec![]).call("iterator", vec![])).clone();
                pc = 127;
            }
            125 => {
                if jvm_cmp(&local_5.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 170; continue 'jvm; }
                pc = 137;
            }
            135 => {
                local_6 = (local_5.call("next", vec![])).clone();
                if !jvm_ref_eq(&local_6.call("getMode", vec![]), &ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPoint$Mode", "DEPOSIT", vec![])) { pc = 164; continue 'jvm; }
                pc = 161;
            }
            158 => {
                local_4 = jvm_iinc(&local_4, 1);
                pc = 167;
            }
            164 => {
                local_3 = jvm_iinc(&local_3, 1);
                pc = 170;
            }
            167 => {
                pc = 125;
            }
            170 => {
                if jvm_cmp(&jvm_bin("+", &local_3, &local_4), &JvmVal::Int(0)).as_i32() <= 0 { pc = 214; continue 'jvm; }
                pc = 180;
            }
            177 => {
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), ctx.call_static("java/lang/Integer", "valueOf", vec![local_3.clone()]));
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(1).as_i32(), ctx.call_static("java/lang/Integer", "valueOf", vec![local_4.clone()]));
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "WHITE", vec![]);
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "builder", vec![]).call("translate", vec![JvmVal::string("mechanical_arm.summary"), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("sendStatus", vec![local_2.clone()]);
                pc = 217;
            }
            214 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentSelection", vec![]);
                ctx.call_static("net/createmod/catnip/platform/CatnipServices", "NETWORK", vec![]).call("sendToServer", vec![JvmVal::obj("com/simibubi/create/content/kinetics/mechanicalArm/ArmPlacementPacket")]);
                ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentSelection", vec![]).call("clear", vec![]);
                // putstatic: mod-own statics are modeled at module scope

                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler.class :: tick (java static)
pub fn tick(ctx: &mut BridgeCtx) {
    let mut local_0: JvmVal = JvmVal::Null;
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_0 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("player")).clone();
                if !jvm_is_null(&local_0) { pc = 12; continue 'jvm; }
                pc = 12;
            }
            11 => {
                return;
            }
            12 => {
                local_1 = (local_0.call("getMainHandItem", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "MECHANICAL_ARM", vec![]).call("isIn", vec![local_1.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 34; continue 'jvm; }
                pc = 28;
            }
            27 => {
                // putstatic: mod-own statics are modeled at module scope

                pc = 59;
            }
            34 => {
                if jvm_ref_eq(&local_1, &ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentItem", vec![])) { pc = 53; continue 'jvm; }
                pc = 44;
            }
            41 => {
                ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentSelection", vec![]).call("clear", vec![]);
                // putstatic: mod-own statics are modeled at module scope

                pc = 56;
            }
            53 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentSelection", vec![]);
                ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "drawOutlines", vec![arg_tmp_0.clone()]);
                pc = 60;
            }
            59 => {
                ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "checkForWrench", vec![local_1.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler.class :: checkForWrench (java static)
pub fn check_for_wrench(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WRENCH", vec![]).call("isIn", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 11; continue 'jvm; }
                pc = 11;
            }
            10 => {
                return;
            }
            11 => {
                local_1 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("hitResult")).clone();
                if jvm_cmp(&jvm_instanceof(&local_1, "net/minecraft/world/phys/BlockHitResult"), &JvmVal::Int(0)).as_i32() == 0 { pc = 33; continue 'jvm; }
                pc = 26;
            }
            25 => {
                local_2 = (local_1).clone();
                pc = 34;
            }
            33 => {
                return;
            }
            34 => {
                local_3 = (local_2.call("getBlockPos", vec![])).clone();
                local_4 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("level").call("getBlockEntity", vec![local_3.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_4, "com/simibubi/create/content/kinetics/mechanicalArm/ArmBlockEntity"), &JvmVal::Int(0)).as_i32() != 0 { pc = 74; continue 'jvm; }
                pc = 62;
            }
            59 => {
                // putstatic: mod-own statics are modeled at module scope

                ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentSelection", vec![]).call("clear", vec![]);
                return;
            }
            74 => {
                if jvm_cmp(&jvm_cmp(&ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "lastBlockPos", vec![]), &JvmVal::Long(-1i64)), &JvmVal::Int(0)).as_i32() == 0 { pc = 95; continue 'jvm; }
                pc = 87;
            }
            84 => {
                if jvm_cmp(&jvm_cmp(&ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "lastBlockPos", vec![]), &local_3.call("asLong", vec![])), &JvmVal::Int(0)).as_i32() == 0 { pc = 147; continue 'jvm; }
                pc = 98;
            }
            95 => {
                ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentSelection", vec![]).call("clear", vec![]);
                local_5 = (local_4).clone();
                local_5.get_field("inputs").call("forEach", vec![JvmVal::Null]);
                local_5.get_field("outputs").call("forEach", vec![JvmVal::Null]);
                // putstatic: mod-own statics are modeled at module scope

                pc = 150;
            }
            147 => {
                if jvm_cmp(&jvm_cmp(&ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "lastBlockPos", vec![]), &JvmVal::Long(-1i64)), &JvmVal::Int(0)).as_i32() == 0 { pc = 163; continue 'jvm; }
                pc = 160;
            }
            157 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentSelection", vec![]);
                ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "drawOutlines", vec![arg_tmp_0.clone()]);
                pc = 163;
            }
            163 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler.class :: drawOutlines (java static)
pub fn draw_outlines(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("iterator", vec![])).clone();
                pc = 8;
            }
            7 => {
                if jvm_cmp(&local_1.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 123; continue 'jvm; }
                pc = 17;
            }
            16 => {
                local_2 = (local_1.call("next", vec![])).clone();
                if jvm_cmp(&local_2.call("isValid", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 42; continue 'jvm; }
                pc = 34;
            }
            33 => {
                local_1.call("remove", vec![]);
                pc = 7;
            }
            42 => {
                local_3 = (local_2.call("getLevel", vec![])).clone();
                local_4 = (local_2.call("getPos", vec![])).clone();
                local_5 = (local_3.call("getBlockState", vec![local_4.clone()])).clone();
                local_6 = (local_5.call("getShape", vec![local_3.clone(), local_4.clone()])).clone();
                if jvm_cmp(&local_6.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 82; continue 'jvm; }
                pc = 82;
            }
            79 => {
                pc = 7;
            }
            82 => {
                local_7 = (local_2.call("getMode", vec![]).call("getColor", vec![])).clone();
                pc = 7;
            }
            123 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler.class :: put (java static)
pub fn put(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler.class :: remove (java static)
pub fn remove(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "getSelected", vec![p0.clone()])).clone();
                if jvm_is_null(&local_1) { pc = 19; continue 'jvm; }
                pc = 12;
            }
            9 => {
                pc = 20;
            }
            19 => {
                return local_1;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler.class :: getSelected (java static)
pub fn get_selected(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (ctx.call_static("com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler", "currentSelection", vec![]).call("iterator", vec![])).clone();
                pc = 10;
            }
            9 => {
                if jvm_cmp(&local_1.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 44; continue 'jvm; }
                pc = 19;
            }
            18 => {
                local_2 = (local_1.call("next", vec![])).clone();
                if jvm_cmp(&local_2.call("getPos", vec![]).call("equals", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 41; continue 'jvm; }
                pc = 40;
            }
            39 => {
                return local_2;
            }
            41 => {
                pc = 9;
            }
            44 => {
                return JvmVal::Null;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: openMenu (java instance)
pub fn open_menu(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return ctx.call_static("java/util/OptionalInt", "empty", vec![]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: getDisplayName (java instance)
pub fn get_display_name(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("block.deployer.damage_source_name"), jvm_array_new(0)]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: getDefaultDimensions (java instance)
pub fn get_default_dimensions(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p_this.call("getDefaultDimensions", vec![p0.clone()]).call("withEyeHeight", vec![JvmVal::Float(0_f32)]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: position (java instance)
pub fn position(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::obj("net/minecraft/world/phys/Vec3");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: getCurrentItemAttackStrengthDelay (java instance)
pub fn get_current_item_attack_strength_delay(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Float(0.015625_f32);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: canEat (java instance)
pub fn can_eat(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: eat (java instance)
pub fn eat(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                p1.call("shrink", vec![JvmVal::Int(1)]);
                return p1;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: canBeAffected (java instance)
pub fn can_be_affected(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: getUUID (java instance)
pub fn get_uuid(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_is_null(&p_this.get_field("owner")) { pc = 14; continue 'jvm; }
                pc = 8;
            }
            7 => {
                pc = 18;
            }
            14 => {
                pc = 18;
            }
            18 => {
                return p_this.get_field("owner");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: deployerHasEyesOnHisFeet (java static)
pub fn deployer_has_eyes_on_his_feet(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p0.call("getEntity", vec![]), "com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer"), &JvmVal::Int(0)).as_i32() == 0 { pc = 22; continue 'jvm; }
                pc = 11;
            }
            10 => {
                p0.call("setNewSize", vec![p0.call("getNewSize", vec![]).call("withEyeHeight", vec![JvmVal::Float(0_f32)])]);
                pc = 22;
            }
            22 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: deployerCollectsDropsFromKilledEntities (java static)
pub fn deployer_collects_drops_from_killed_entities(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getSource", vec![])).clone();
                local_2 = (local_1.call("getEntity", vec![])).clone();
                if jvm_is_null(&local_2) { pc = 46; continue 'jvm; }
                pc = 15;
            }
            14 => {
                if jvm_cmp(&jvm_instanceof(&local_2, "com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer"), &JvmVal::Int(0)).as_i32() == 0 { pc = 46; continue 'jvm; }
                pc = 22;
            }
            21 => {
                local_3 = (local_2).clone();
                local_3.call("forEach", vec![JvmVal::Null]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                pc = 46;
            }
            46 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: doesEmitEquipEvent (java instance)
pub fn does_emit_equip_event(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: remove (java instance)
pub fn remove_2(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_is_null(&p_this.get_field("blockBreakingProgress")) { pc = 39; continue 'jvm; }
                pc = 8;
            }
            7 => {
                if jvm_cmp(&p_this.call("level", vec![]).get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 39; continue 'jvm; }
                pc = 18;
            }
            17 => {
                p_this.call("level", vec![]).call("destroyBlockProgress", vec![p_this.call("getId", vec![]), p_this.get_field("blockBreakingProgress").call("getKey", vec![]), JvmVal::Int(-1)]);
                pc = 40;
            }
            39 => {
                p_this.call("remove", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: deployerKillsDoNotSpawnXP (java static)
pub fn deployer_kills_do_not_spawn_xp(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p0.call("getAttackingPlayer", vec![]), "com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer"), &JvmVal::Int(0)).as_i32() == 0 { pc = 15; continue 'jvm; }
                pc = 11;
            }
            10 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                pc = 15;
            }
            15 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: entitiesDontRetaliate (java static)
pub fn entities_dont_retaliate(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p0.call("getOriginalAboutToBeSetTarget", vec![]), "com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer"), &JvmVal::Int(0)).as_i32() != 0 { pc = 11; continue 'jvm; }
                pc = 11;
            }
            10 => {
                return;
            }
            11 => {
                local_1 = (p0.call("getEntity", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_1, "net/minecraft/world/entity/Mob"), &JvmVal::Int(0)).as_i32() == 0 { pc = 31; continue 'jvm; }
                pc = 24;
            }
            23 => {
                local_2 = (local_1).clone();
                pc = 32;
            }
            31 => {
                return;
            }
            32 => {
                local_3 = (ctx.call_static("com/simibubi/create/infrastructure/config/AllConfigs", "server", vec![]).get_field("kinetics").get_field("ignoreDeployerAttacks").call("get", vec![])).clone();
                match jvm_array_load(&ctx.call_static("com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer$1", "$SwitchMap$com$simibubi$create$infrastructure$config$CKinetics$DeployerAggroSetting", vec![]), local_3.call("ordinal", vec![]).as_i32()).as_i32() {
                    1 => { pc = 84; }
                    2 => { pc = 92; }
                    _ => { pc = 104; }
                }
            }
            84 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                pc = 104;
            }
            92 => {
                if jvm_cmp(&jvm_instanceof(&local_2, "net/minecraft/world/entity/monster/Creeper"), &JvmVal::Int(0)).as_i32() == 0 { pc = 104; continue 'jvm; }
                pc = 100;
            }
            99 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                pc = 104;
            }
            104 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer.class :: lambda$deployerCollectsDropsFromKilledEntities$0 (java static)
pub fn lambda_deployer_collects_drops_from_killed_entities_0(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                p0.call("getInventory", vec![]).call("placeItemBackInInventory", vec![p1.call("getItem", vec![])]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/deployer/ManualApplicationRecipe.class :: manualApplicationRecipesApplyInWorld (java static)
pub fn manual_application_recipes_apply_in_world(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut local_14: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getLevel", vec![])).clone();
                local_2 = (p0.call("getItemStack", vec![])).clone();
                local_3 = (p0.call("getPos", vec![])).clone();
                local_4 = (local_1.call("getBlockState", vec![local_3.clone()])).clone();
                if jvm_cmp(&local_2.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 30; continue 'jvm; }
                pc = 30;
            }
            29 => {
                return;
            }
            30 => {
                if jvm_cmp(&local_4.call("isAir", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 39; continue 'jvm; }
                pc = 39;
            }
            38 => {
                return;
            }
            39 => {
                if jvm_cmp(&p0.call("isCanceled", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 47; continue 'jvm; }
                pc = 47;
            }
            46 => {
                return;
            }
            47 => {
                local_5 = (ctx.call_static("com/simibubi/create/AllRecipeTypes", "ITEM_APPLICATION", vec![]).call("getType", vec![])).clone();
                local_6 = (local_2.call("filter", vec![JvmVal::Null]).call("findFirst", vec![])).clone();
                if jvm_cmp(&local_6.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 98; continue 'jvm; }
                pc = 98;
            }
            97 => {
                return;
            }
            98 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                if jvm_cmp(&local_1.call("isClientSide", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 118; continue 'jvm; }
                pc = 118;
            }
            117 => {
                return;
            }
            118 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/sounds/SoundSource", "PLAYERS", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/sounds/SoundEvents", "COPPER_BREAK", vec![]);
                local_1.call("playSound", vec![JvmVal::Null, local_3.clone(), arg_tmp_1.clone(), arg_tmp_0.clone(), JvmVal::Float(1_f32), JvmVal::Float(1.45_f32)]);
                local_7 = (local_6.call("get", vec![]).call("value", vec![])).clone();
                local_8 = (local_7.call("transformBlock", vec![local_4.clone(), local_1.get_field("random")])).clone();
                local_3.call("forEach", vec![JvmVal::Null]);
                if jvm_is_null(&p0.call("getEntity", vec![])) { pc = 220; continue 'jvm; }
                pc = 207;
            }
            206 => {
                if jvm_cmp(&p0.call("getEntity", vec![]).call("isCreative", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 220; continue 'jvm; }
                pc = 217;
            }
            216 => {
                pc = 221;
            }
            220 => {
                pc = 223;
            }
            221 => {
                local_9 = (JvmVal::Int(0)).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/component/DataComponents", "UNBREAKABLE", vec![]);
                local_10 = (local_2.call("has", vec![arg_tmp_0.clone()])).clone();
                if jvm_cmp(&local_7.call("shouldKeepHeldItem", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 245; continue 'jvm; }
                pc = 242;
            }
            240 => {
                if jvm_cmp(&local_9, &JvmVal::Int(0)).as_i32() == 0 { pc = 249; continue 'jvm; }
                pc = 246;
            }
            245 => {
                pc = 250;
            }
            249 => {
                pc = 252;
            }
            250 => {
                local_11 = (JvmVal::Int(0)).clone();
                if jvm_cmp(&local_10, &JvmVal::Int(0)).as_i32() != 0 { pc = 348; continue 'jvm; }
                pc = 259;
            }
            257 => {
                if jvm_cmp(&local_11, &JvmVal::Int(0)).as_i32() != 0 { pc = 348; continue 'jvm; }
                pc = 263;
            }
            262 => {
                if jvm_cmp(&local_2.call("getMaxDamage", vec![]), &JvmVal::Int(0)).as_i32() <= 0 { pc = 284; continue 'jvm; }
                pc = 270;
            }
            269 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/entity/EquipmentSlot", "MAINHAND", vec![]);
                local_2.call("hurtAndBreak", vec![JvmVal::Int(1), p0.call("getEntity", vec![]), arg_tmp_0.clone()]);
                pc = 348;
            }
            284 => {
                local_12 = (p0.call("getEntity", vec![])).clone();
                local_13 = (p0.call("getHand", vec![])).clone();
                local_14 = (local_2.call("getCraftingRemainingItem", vec![])).clone();
                local_2.call("shrink", vec![JvmVal::Int(1)]);
                if jvm_cmp(&local_2.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 326; continue 'jvm; }
                pc = 316;
            }
            314 => {
                local_12.call("setItemInHand", vec![local_13.clone(), local_14.clone()]);
                pc = 348;
            }
            326 => {
                if jvm_cmp(&local_12.call("getInventory", vec![]).call("add", vec![local_14.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 348; continue 'jvm; }
                pc = 341;
            }
            339 => {
                pc = 349;
            }
            348 => {
                ctx.call_static("com/simibubi/create/content/kinetics/deployer/ManualApplicationRecipe", "awardAdvancements", vec![p0.call("getEntity", vec![]), local_8.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/deployer/ManualApplicationRecipe.class :: awardAdvancements (java static)
pub fn award_advancements(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (JvmVal::Null).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "ANDESITE_CASING", vec![]).call("has", vec![p1.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 19; continue 'jvm; }
                pc = 15;
            }
            12 => {
                local_2 = (ctx.call_static("com/simibubi/create/foundation/advancement/AllAdvancements", "ANDESITE_CASING", vec![])).clone();
                pc = 71;
            }
            19 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "BRASS_CASING", vec![]).call("has", vec![p1.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 36; continue 'jvm; }
                pc = 32;
            }
            29 => {
                local_2 = (ctx.call_static("com/simibubi/create/foundation/advancement/AllAdvancements", "BRASS_CASING", vec![])).clone();
                pc = 71;
            }
            36 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "COPPER_CASING", vec![]).call("has", vec![p1.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 53; continue 'jvm; }
                pc = 49;
            }
            46 => {
                local_2 = (ctx.call_static("com/simibubi/create/foundation/advancement/AllAdvancements", "COPPER_CASING", vec![])).clone();
                pc = 71;
            }
            53 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "RAILWAY_CASING", vec![]).call("has", vec![p1.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 70; continue 'jvm; }
                pc = 66;
            }
            63 => {
                local_2 = (ctx.call_static("com/simibubi/create/foundation/advancement/AllAdvancements", "TRAIN_CASING", vec![])).clone();
                pc = 71;
            }
            70 => {
                return;
            }
            71 => {
                local_2.call("awardTo", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/deployer/ManualApplicationRecipe.class :: asDeploying (java static)
pub fn as_deploying(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("value", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllRecipeTypes", "CAN_BE_AUTOMATED", vec![]).call("test", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 33; continue 'jvm; }
                pc = 21;
            }
            20 => {
                pc = 37;
            }
            33 => {
                pc = 38;
            }
            37 => {
                local_2 = (p0.call("id", vec![])).clone();
                local_3 = (JvmVal::obj("com/simibubi/create/content/kinetics/deployer/ItemApplicationRecipe$Builder").call("require", vec![local_1.get_field("ingredients").call("get", vec![JvmVal::Int(0)])]).call("require", vec![local_1.get_field("ingredients").call("get", vec![JvmVal::Int(1)])])).clone();
                local_4 = (local_1.get_field("results").call("iterator", vec![])).clone();
                pc = 97;
            }
            95 => {
                if jvm_cmp(&local_4.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 127; continue 'jvm; }
                pc = 107;
            }
            105 => {
                local_5 = (local_4.call("next", vec![])).clone();
                pc = 95;
            }
            127 => {
                if jvm_cmp(&local_1.call("shouldKeepHeldItem", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 139; continue 'jvm; }
                pc = 135;
            }
            134 => {
                pc = 142;
            }
            139 => {
                return JvmVal::obj("net/minecraft/world/item/crafting/RecipeHolder");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/ManualApplicationRecipe.class :: testBlock (java instance)
pub fn test_block(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p_this.get_field("ingredients").call("get", vec![JvmVal::Int(0)]).call("test", vec![JvmVal::obj("net/minecraft/world/item/ItemStack")]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/ManualApplicationRecipe.class :: transformBlock (java instance)
pub fn transform_block(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p_this.get_field("results").call("get", vec![JvmVal::Int(0)])).clone();
                local_4 = (local_3.call("rollOutput", vec![p1.clone()])).clone();
                local_6 = (local_4.call("getItem", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_6, "net/minecraft/world/item/BlockItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 54; continue 'jvm; }
                pc = 36;
            }
            34 => {
                local_5 = (local_6).clone();
                return ctx.call_static("com/simibubi/create/foundation/utility/BlockHelper", "copyProperties", vec![p0.clone(), local_5.call("getBlock", vec![]).call("defaultBlockState", vec![])]);
            }
            54 => {
                return ctx.call_static("net/minecraft/world/level/block/Blocks", "AIR", vec![]).call("defaultBlockState", vec![]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/ManualApplicationRecipe.class :: rollResults (java instance)
pub fn roll_results(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p_this.call("rollResults", vec![p_this.call("getRollableResultsExceptBlock", vec![]), p0.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/ManualApplicationRecipe.class :: getRollableResultsExceptBlock (java instance)
pub fn get_rollable_results_except_block(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p_this.get_field("results").call("get", vec![JvmVal::Int(0)])).clone();
                if jvm_cmp(&jvm_instanceof(&local_1.call("getStack", vec![]).call("getItem", vec![]), "net/minecraft/world/item/BlockItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 41; continue 'jvm; }
                pc = 26;
            }
            25 => {
                return p_this.get_field("results").call("subList", vec![JvmVal::Int(1), p_this.get_field("results").call("size", vec![])]);
            }
            41 => {
                return p_this.get_field("results");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/deployer/ManualApplicationRecipe.class :: lambda$manualApplicationRecipesApplyInWorld$1 (java static)
pub fn lambda_manual_application_recipes_apply_in_world_1(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("net/minecraft/world/level/block/Block", "popResource", vec![p0.clone(), p1.clone(), p2.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/deployer/ManualApplicationRecipe.class :: lambda$manualApplicationRecipesApplyInWorld$0 (java static)
pub fn lambda_manual_application_recipes_apply_in_world_0(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p2.call("value", vec![])).clone();
                if jvm_cmp(&local_3.call("testBlock", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 38; continue 'jvm; }
                pc = 17;
            }
            16 => {
                if jvm_cmp(&local_3.get_field("ingredients").call("get", vec![JvmVal::Int(1)]).call("test", vec![p1.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 38; continue 'jvm; }
                pc = 35;
            }
            34 => {
                pc = 39;
            }
            38 => {
                pc = 39;
            }
            39 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/crank/ValveHandleBlock.class :: copper (java static)
pub fn copper(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::obj("com/simibubi/create/content/kinetics/crank/ValveHandleBlock");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/crank/ValveHandleBlock.class :: dyed (java static)
pub fn dyed(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::obj("com/simibubi/create/content/kinetics/crank/ValveHandleBlock");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/crank/ValveHandleBlock.class :: getShape (java instance)
pub fn get_shape(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/crank/ValveHandleBlock", "FACING", vec![]);
                return ctx.call_static("com/simibubi/create/AllShapes", "VALVE_HANDLE", vec![]).call("get", vec![p0.call("getValue", vec![arg_tmp_0.clone()])]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/crank/ValveHandleBlock.class :: onBlockActivated (java static)
pub fn on_block_activated(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getPos", vec![])).clone();
                local_2 = (p0.call("getLevel", vec![])).clone();
                local_3 = (p0.call("getEntity", vec![])).clone();
                local_4 = (local_2.call("getBlockState", vec![local_1.clone()])).clone();
                local_6 = (local_4.call("getBlock", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_6, "com/simibubi/create/content/kinetics/crank/ValveHandleBlock"), &JvmVal::Int(0)).as_i32() == 0 { pc = 47; continue 'jvm; }
                pc = 39;
            }
            37 => {
                local_5 = (local_6).clone();
                pc = 48;
            }
            47 => {
                return;
            }
            48 => {
                if jvm_cmp(&local_3.call("mayBuild", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 56; continue 'jvm; }
                pc = 56;
            }
            55 => {
                return;
            }
            56 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WRENCH", vec![]).call("isIn", vec![local_3.call("getItemInHand", vec![p0.call("getHand", vec![])])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 81; continue 'jvm; }
                pc = 74;
            }
            73 => {
                if jvm_cmp(&local_3.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 81; continue 'jvm; }
                pc = 81;
            }
            80 => {
                return;
            }
            81 => {
                if jvm_cmp(&local_5.call("clicked", vec![local_2.clone(), local_1.clone(), local_4.clone(), local_3.clone(), p0.call("getHand", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 110; continue 'jvm; }
                pc = 99;
            }
            98 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                pc = 110;
            }
            110 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/crank/ValveHandleBlock.class :: onRemove (java instance)
pub fn on_remove(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p3.call("getBlock", vec![]), "com/simibubi/create/content/kinetics/crank/ValveHandleBlock"), &JvmVal::Int(0)).as_i32() != 0 { pc = 22; continue 'jvm; }
                pc = 12;
            }
            11 => {
                p_this.call("onRemove", vec![p0.clone(), p1.clone(), p2.clone(), p3.clone(), p4.clone()]);
                pc = 22;
            }
            22 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/crank/ValveHandleBlock.class :: clicked (java instance)
pub fn clicked(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_6 = (p3.call("getItemInHand", vec![p4.clone()])).clone();
                local_7 = (ctx.call_static("net/minecraft/world/item/DyeColor", "getColor", vec![local_6.clone()])).clone();
                if jvm_is_null(&local_7) { pc = 60; continue 'jvm; }
                pc = 23;
            }
            21 => {
                if jvm_ref_eq(&local_7, &p_this.get_field("color")) { pc = 60; continue 'jvm; }
                pc = 31;
            }
            30 => {
                if jvm_cmp(&p0.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 58; continue 'jvm; }
                pc = 38;
            }
            37 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllBlocks", "DYED_VALVE_HANDLES", vec![]).call("get", vec![local_7.clone()]).call("getDefaultState", vec![]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/BlockHelper", "copyProperties", vec![p2.clone(), arg_tmp_0.clone()]);
                pc = 59;
            }
            58 => {
                return JvmVal::Int(1);
            }
            60 => {
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/crank/ValveHandleBlock.class :: useItemOn (java instance)
pub fn use_item_on(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal, p5: JvmVal, p6: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut p5: JvmVal = p5.clone();
    let mut p6: JvmVal = p6.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return ctx.call_static("net/minecraft/world/ItemInteractionResult", "PASS_TO_DEFAULT_BLOCK_INTERACTION", vec![]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/crank/ValveHandleBlock.class :: getBlockEntityType (java instance)
pub fn get_block_entity_type(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return ctx.call_static("com/simibubi/create/AllBlockEntityTypes", "VALVE_HANDLE", vec![]).call("get", vec![]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/crank/ValveHandleBlock.class :: getRotationSpeed (java instance)
pub fn get_rotation_speed(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Int(32);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/crank/ValveHandleBlock.class :: lambda$clicked$0 (java static)
pub fn lambda_clicked_0(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p1, "com/simibubi/create/content/kinetics/crank/ValveHandleBlockEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 29; continue 'jvm; }
                pc = 8;
            }
            7 => {
                local_2 = (p1).clone();
                if jvm_cmp(&local_2.call("activate", vec![p0.call("isShiftKeyDown", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 29; continue 'jvm; }
                pc = 26;
            }
            23 => {
                pc = 32;
            }
            29 => {
                pc = 32;
            }
            32 => {
                return ctx.call_static("net/minecraft/world/InteractionResult", "PASS", vec![]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler.class :: onRightClick (java static)
pub fn on_right_click(ctx: &mut BridgeCtx) -> JvmVal {
    let mut local_0: JvmVal = JvmVal::Null;
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_0 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "isChain", vec![local_0.get_field("player").call("getMainHandItem", vec![])]), &JvmVal::Int(0)).as_i32() != 0 { pc = 19; continue 'jvm; }
                pc = 18;
            }
            17 => {
                return JvmVal::Int(0);
            }
            19 => {
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![])) { pc = 27; continue 'jvm; }
                pc = 26;
            }
            25 => {
                return JvmVal::Int(0);
            }
            27 => {
                local_1 = (JvmVal::Int(0)).clone();
                local_3 = (local_0.get_field("hitResult")).clone();
                if jvm_cmp(&jvm_instanceof(&local_3, "net/minecraft/world/phys/BlockHitResult"), &JvmVal::Int(0)).as_i32() == 0 { pc = 75; continue 'jvm; }
                pc = 42;
            }
            41 => {
                local_2 = (local_3).clone();
                if jvm_ref_eq(&local_2.call("getType", vec![]), &ctx.call_static("net/minecraft/world/phys/HitResult$Type", "MISS", vec![])) { pc = 75; continue 'jvm; }
                pc = 57;
            }
            56 => {
                if jvm_cmp(&jvm_instanceof(&local_0.get_field("level").call("getBlockEntity", vec![local_2.call("getBlockPos", vec![])]), "com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorBlockEntity"), &JvmVal::Int(0)).as_i32() != 0 { pc = 75; continue 'jvm; }
                pc = 74;
            }
            73 => {
                local_1 = (JvmVal::Int(1)).clone();
                pc = 76;
            }
            75 => {
                if jvm_cmp(&local_0.get_field("player").call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 91; continue 'jvm; }
                pc = 86;
            }
            85 => {
                if jvm_cmp(&local_1, &JvmVal::Int(0)).as_i32() != 0 { pc = 91; continue 'jvm; }
                pc = 90;
            }
            89 => {
                return JvmVal::Int(0);
            }
            91 => {
                // putstatic: mod-own statics are modeled at module scope

                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("chain_conveyor.selection_cleared"), jvm_array_new(0)]).call("sendStatus", vec![local_0.get_field("player")]);
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler.class :: onItemUsedOnBlock (java static)
pub fn on_item_used_on_block(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getItemStack", vec![])).clone();
                local_2 = (p0.call("getPos", vec![])).clone();
                local_3 = (p0.call("getLevel", vec![])).clone();
                local_4 = (p0.call("getEntity", vec![])).clone();
                local_5 = (local_3.call("getBlockState", vec![local_2.clone()])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "CHAIN_CONVEYOR", vec![]).call("has", vec![local_5.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 40; continue 'jvm; }
                pc = 40;
            }
            39 => {
                return;
            }
            40 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "isChain", vec![local_1.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 48; continue 'jvm; }
                pc = 48;
            }
            47 => {
                return;
            }
            48 => {
                if jvm_cmp(&local_4.call("mayBuild", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 64; continue 'jvm; }
                pc = 58;
            }
            56 => {
                if jvm_cmp(&jvm_instanceof(&local_4, "net/neoforged/neoforge/common/util/FakePlayer"), &JvmVal::Int(0)).as_i32() == 0 { pc = 65; continue 'jvm; }
                pc = 65;
            }
            64 => {
                return;
            }
            65 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "CONSUME", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                if jvm_cmp(&local_3.call("isClientSide", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 85; continue 'jvm; }
                pc = 85;
            }
            84 => {
                return;
            }
            85 => {
                local_7 = (local_3.call("getBlockEntity", vec![local_2.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_7, "com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorBlockEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 159; continue 'jvm; }
                pc = 102;
            }
            100 => {
                local_6 = (local_7).clone();
                if jvm_cmp(&local_6.get_field("connections").call("size", vec![]), &ctx.call_static("com/simibubi/create/infrastructure/config/AllConfigs", "server", vec![]).get_field("kinetics").get_field("maxChainConveyorConnections").call("get", vec![]).call("intValue", vec![])).as_i32() < 0 { pc = 159; continue 'jvm; }
                pc = 140;
            }
            138 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("chain_conveyor.cannot_add_more_connections"), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("sendStatus", vec![local_4.clone()]);
                return;
            }
            159 => {
                if jvm_is_null(&ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![])) { pc = 175; continue 'jvm; }
                pc = 168;
            }
            165 => {
                if jvm_ref_eq(&ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstDim", vec![]), &local_3.call("dimension", vec![])) { pc = 196; continue 'jvm; }
                pc = 176;
            }
            175 => {
                // putstatic: mod-own statics are modeled at module scope

                // putstatic: mod-own statics are modeled at module scope

                local_4.call("swing", vec![p0.call("getHand", vec![])]);
                return;
            }
            196 => {
                local_6 = (ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "validateAndConnect", vec![local_3.clone(), local_2.clone(), local_4.clone(), local_1.clone(), JvmVal::Int(0)])).clone();
                // putstatic: mod-own statics are modeled at module scope

                if jvm_cmp(&local_6, &JvmVal::Int(0)).as_i32() != 0 { pc = 227; continue 'jvm; }
                pc = 219;
            }
            216 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "DENY", vec![]).call("play", vec![local_3.clone(), local_4.clone(), local_2.clone()]);
                return;
            }
            227 => {
                local_7 = (ctx.call_static("net/minecraft/world/level/block/Blocks", "CHAIN", vec![]).call("defaultBlockState", vec![]).call("getSoundType", vec![])).clone();
                if jvm_is_null(&local_7) { pc = 276; continue 'jvm; }
                pc = 244;
            }
            243 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/sounds/SoundSource", "BLOCKS", vec![]);
                local_3.call("playSound", vec![local_4.clone(), local_2.clone(), local_7.call("getPlaceSound", vec![]), arg_tmp_0.clone(), jvm_bin("/", &jvm_bin("+", &local_7.call("getVolume", vec![]), &JvmVal::Float(1_f32)), &JvmVal::Float(2_f32)), jvm_bin("*", &local_7.call("getPitch", vec![]), &JvmVal::Float(0.8_f32))]);
                pc = 276;
            }
            276 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler.class :: isChain (java static)
pub fn is_chain(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/item/Items", "CHAIN", vec![]);
                return p0.call("is", vec![arg_tmp_0.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler.class :: clientTick (java static)
pub fn client_tick(ctx: &mut BridgeCtx) {
    let mut local_0: JvmVal = JvmVal::Null;
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![])) { pc = 7; continue 'jvm; }
                pc = 7;
            }
            6 => {
                return;
            }
            7 => {
                local_0 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("player")).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                local_1 = (local_0.call("level", vec![]).call("getBlockEntity", vec![arg_tmp_0.clone()])).clone();
                if !jvm_ref_eq(&ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstDim", vec![]), &local_0.call("level", vec![]).call("dimension", vec![])) { pc = 45; continue 'jvm; }
                pc = 39;
            }
            38 => {
                if jvm_cmp(&jvm_instanceof(&local_1, "com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorBlockEntity"), &JvmVal::Int(0)).as_i32() != 0 { pc = 63; continue 'jvm; }
                pc = 46;
            }
            45 => {
                // putstatic: mod-own statics are modeled at module scope

                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("chain_conveyor.selection_cleared"), jvm_array_new(0)]).call("sendStatus", vec![local_0.clone()]);
                return;
            }
            63 => {
                local_2 = (local_0.call("getMainHandItem", vec![])).clone();
                local_3 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("hitResult")).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "isChain", vec![local_2.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 95; continue 'jvm; }
                pc = 83;
            }
            82 => {
                local_2 = (local_0.call("getOffhandItem", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "isChain", vec![local_2.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 95; continue 'jvm; }
                pc = 95;
            }
            94 => {
                return;
            }
            95 => {
                if jvm_is_null(&local_3) { pc = 109; continue 'jvm; }
                pc = 100;
            }
            99 => {
                if jvm_ref_eq(&local_3.call("getType", vec![]), &ctx.call_static("net/minecraft/world/phys/HitResult$Type", "BLOCK", vec![])) { pc = 122; continue 'jvm; }
                pc = 112;
            }
            109 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "highlightConveyor", vec![arg_tmp_0.clone(), JvmVal::Int(16777215), JvmVal::string("chain_connect")]);
                return;
            }
            122 => {
                local_4 = (local_0.call("level", vec![])).clone();
                local_5 = (local_3).clone();
                local_6 = (local_5.call("getBlockPos", vec![])).clone();
                local_7 = (local_4.call("getBlockState", vec![local_6.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                if jvm_cmp(&local_6.call("equals", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 188; continue 'jvm; }
                pc = 164;
            }
            161 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "highlightConveyor", vec![arg_tmp_0.clone(), JvmVal::Int(16777215), JvmVal::string("chain_connect")]);
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("chain_conveyor.select_second"), jvm_array_new(0)]).call("sendStatus", vec![local_0.clone()]);
                return;
            }
            188 => {
                if jvm_cmp(&jvm_instanceof(&local_7.call("getBlock", vec![]), "com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorBlock"), &JvmVal::Int(0)).as_i32() != 0 { pc = 212; continue 'jvm; }
                pc = 202;
            }
            199 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "highlightConveyor", vec![arg_tmp_0.clone(), JvmVal::Int(16777215), JvmVal::string("chain_connect")]);
                return;
            }
            212 => {
                local_8 = (ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "validateAndConnect", vec![local_4.clone(), local_6.clone(), local_0.clone(), local_2.clone(), JvmVal::Int(1)])).clone();
                if jvm_cmp(&local_8, &JvmVal::Int(0)).as_i32() == 0 { pc = 249; continue 'jvm; }
                pc = 232;
            }
            229 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "GREEN", vec![]);
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("chain_conveyor.valid_connection"), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("sendStatus", vec![local_0.clone()]);
                pc = 251;
            }
            249 => {
                if jvm_cmp(&local_8, &JvmVal::Int(0)).as_i32() == 0 { pc = 260; continue 'jvm; }
                pc = 257;
            }
            254 => {
                pc = 263;
            }
            260 => {
                pc = 265;
            }
            263 => {
                local_9 = (JvmVal::Int(15359019)).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "highlightConveyor", vec![arg_tmp_0.clone(), local_9.clone(), JvmVal::string("chain_connect")]);
                ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "highlightConveyor", vec![local_6.clone(), local_9.clone(), JvmVal::string("chain_connect_to")]);
                local_10 = (ctx.call_static("net/minecraft/world/phys/Vec3", "atCenterOf", vec![local_6.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                local_11 = (ctx.call_static("net/minecraft/world/phys/Vec3", "atCenterOf", vec![arg_tmp_0.clone()])).clone();
                local_12 = (local_10.call("subtract", vec![local_11.clone()])).clone();
                if jvm_cmp(&jvm_cmp(&local_12.call("length", vec![]), &JvmVal::Double(1_f64)), &JvmVal::Int(0)).as_i32() >= 0 { pc = 321; continue 'jvm; }
                pc = 321;
            }
            320 => {
                return;
            }
            321 => {
                local_10 = (local_10.call("subtract", vec![local_12.call("normalize", vec![]).call("scale", vec![JvmVal::Double(0.5_f64)])])).clone();
                local_11 = (local_11.call("add", vec![local_12.call("normalize", vec![]).call("scale", vec![JvmVal::Double(0.5_f64)])])).clone();
                local_13 = (local_12.call("cross", vec![JvmVal::obj("net/minecraft/world/phys/Vec3")]).call("normalize", vec![]).call("scale", vec![JvmVal::Double(0.875_f64)])).clone();
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler.class :: highlightConveyor (java static)
pub fn highlight_conveyor(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (ctx.call_static("net/createmod/catnip/data/Iterate", "zeroAndOne", vec![])).clone();
                local_4 = (jvm_array_length(&local_3)).clone();
                local_5 = (JvmVal::Int(0)).clone();
                pc = 13;
            }
            11 => {
                if jvm_cmp(&local_5, &local_4).as_i32() >= 0 { pc = 168; continue 'jvm; }
                pc = 19;
            }
            18 => {
                local_6 = (jvm_array_load(&local_3, local_5.as_i32())).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/Direction$Axis", "Y", vec![]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/phys/Vec3", "atBottomCenterOf", vec![p0.clone()]);
                local_7 = (ctx.call_static("net/createmod/catnip/math/VecHelper", "rotate", vec![JvmVal::obj("net/minecraft/world/phys/Vec3"), JvmVal::Double(-22.5_f64), arg_tmp_0.clone()]).call("add", vec![arg_tmp_0.clone()])).clone();
                local_8 = (JvmVal::Int(0)).clone();
                pc = 69;
            }
            67 => {
                if jvm_cmp(&local_8, &JvmVal::Int(8)).as_i32() >= 0 { pc = 162; continue 'jvm; }
                pc = 77;
            }
            74 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/Direction$Axis", "Y", vec![]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/phys/Vec3", "atBottomCenterOf", vec![p0.clone()]);
                local_9 = (ctx.call_static("net/createmod/catnip/math/VecHelper", "rotate", vec![JvmVal::obj("net/minecraft/world/phys/Vec3"), jvm_bin("+", &JvmVal::Double(22.5_f64), &jvm_conv(&jvm_bin("*", &local_8, &JvmVal::Int(45)), "D")), arg_tmp_0.clone()]).call("add", vec![arg_tmp_0.clone()])).clone();
                local_7 = (local_9).clone();
                local_8 = jvm_iinc(&local_8, 1);
                pc = 67;
            }
            162 => {
                local_5 = jvm_iinc(&local_5, 1);
                pc = 11;
            }
            168 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler.class :: validateAndConnect (java static)
pub fn validate_and_connect(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p4, &JvmVal::Int(0)).as_i32() != 0 { pc = 27; continue 'jvm; }
                pc = 6;
            }
            5 => {
                if jvm_cmp(&p2.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 27; continue 'jvm; }
                pc = 14;
            }
            12 => {
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("chain_conveyor.selection_cleared"), jvm_array_new(0)]).call("sendStatus", vec![p2.clone()]);
                return JvmVal::Int(0);
            }
            27 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                if jvm_cmp(&p1.call("equals", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 39; continue 'jvm; }
                pc = 38;
            }
            37 => {
                return JvmVal::Int(0);
            }
            39 => {
    let arg_tmp_0 = jvm_conv(&ctx.call_static("com/simibubi/create/infrastructure/config/AllConfigs", "server", vec![]).get_field("kinetics").get_field("maxChainConveyorLength").call("get", vec![]).call("intValue", vec![]), "D");
    let arg_tmp_1 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                if jvm_cmp(&p1.call("closerThan", vec![arg_tmp_1.clone(), arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 75; continue 'jvm; }
                pc = 71;
            }
            68 => {
                return ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "fail", vec![JvmVal::string("chain_conveyor.too_far")]);
            }
            75 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                if jvm_cmp(&p1.call("closerThan", vec![arg_tmp_0.clone(), JvmVal::Double(2.5_f64)]), &JvmVal::Int(0)).as_i32() == 0 { pc = 95; continue 'jvm; }
                pc = 91;
            }
            88 => {
                return ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "fail", vec![JvmVal::string("chain_conveyor.too_close")]);
            }
            95 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                local_5 = (ctx.call_static("net/minecraft/world/phys/Vec3", "atLowerCornerOf", vec![p1.call("subtract", vec![arg_tmp_0.clone()])])).clone();
                local_6 = (jvm_bin("-", &local_5.call("multiply", vec![JvmVal::Double(1_f64), JvmVal::Double(0_f64), JvmVal::Double(1_f64)]).call("length", vec![]), &JvmVal::Double(1.5_f64))).clone();
                if jvm_cmp(&jvm_cmp(&local_6, &JvmVal::Double(0_f64)), &JvmVal::Int(0)).as_i32() > 0 { pc = 138; continue 'jvm; }
                pc = 134;
            }
            131 => {
                return ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "fail", vec![JvmVal::string("chain_conveyor.cannot_connect_vertically")]);
            }
            138 => {
                if jvm_cmp(&jvm_cmp(&jvm_bin("/", &ctx.call_static("java/lang/Math", "abs", vec![local_5.get_field("y")]), &local_6), &JvmVal::Double(1_f64)), &JvmVal::Int(0)).as_i32() <= 0 { pc = 161; continue 'jvm; }
                pc = 157;
            }
            154 => {
                return ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "fail", vec![JvmVal::string("chain_conveyor.too_steep")]);
            }
            161 => {
                local_8 = (ctx.call_static("com/simibubi/create/AllBlocks", "CHAIN_CONVEYOR", vec![]).call("get", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                local_9 = (local_8.call("getBlockEntity", vec![p0.clone(), arg_tmp_0.clone()])).clone();
                local_10 = (local_8.call("getBlockEntity", vec![p0.clone(), p1.clone()])).clone();
                if jvm_cmp(&local_10.get_field("connections").call("size", vec![]), &ctx.call_static("com/simibubi/create/infrastructure/config/AllConfigs", "server", vec![]).get_field("kinetics").get_field("maxChainConveyorConnections").call("get", vec![]).call("intValue", vec![])).as_i32() < 0 { pc = 235; continue 'jvm; }
                pc = 231;
            }
            229 => {
                return ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "fail", vec![JvmVal::string("chain_conveyor.cannot_add_more_connections")]);
            }
            235 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]).call("subtract", vec![p1.clone()]);
                if jvm_cmp(&local_10.get_field("connections").call("contains", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 262; continue 'jvm; }
                pc = 258;
            }
            255 => {
                return ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "fail", vec![JvmVal::string("chain_conveyor.already_connected")]);
            }
            262 => {
                if jvm_is_null(&local_9) { pc = 272; continue 'jvm; }
                pc = 269;
            }
            267 => {
                if !jvm_is_null(&local_10) { pc = 279; continue 'jvm; }
                pc = 275;
            }
            272 => {
                return ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "fail", vec![JvmVal::string("chain_conveyor.blocks_invalid")]);
            }
            279 => {
                if jvm_cmp(&p2.call("isCreative", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 336; continue 'jvm; }
                pc = 287;
            }
            286 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                local_11 = (ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorBlockEntity", "getChainCost", vec![p1.call("subtract", vec![arg_tmp_0.clone()])])).clone();
                local_12 = (ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorBlockEntity", "getChainsFromInventory", vec![p2.clone(), p3.clone(), local_11.clone(), JvmVal::Int(1)])).clone();
                if jvm_cmp(&p4, &JvmVal::Int(0)).as_i32() == 0 { pc = 324; continue 'jvm; }
                pc = 314;
            }
            313 => {
                ctx.call_static("com/simibubi/create/content/equipment/blueprint/BlueprintOverlayRenderer", "displayChainRequirements", vec![p3.call("getItem", vec![]), local_11.clone(), local_12.clone()]);
                pc = 326;
            }
            324 => {
                if jvm_cmp(&local_12, &JvmVal::Int(0)).as_i32() != 0 { pc = 336; continue 'jvm; }
                pc = 332;
            }
            329 => {
                return ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "fail", vec![JvmVal::string("chain_conveyor.not_enough_chains")]);
            }
            336 => {
                if jvm_cmp(&p4, &JvmVal::Int(0)).as_i32() == 0 { pc = 343; continue 'jvm; }
                pc = 342;
            }
            341 => {
                return JvmVal::Int(1);
            }
            343 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler", "firstPos", vec![]);
                ctx.call_static("net/createmod/catnip/platform/CatnipServices", "NETWORK", vec![]).call("sendToServer", vec![JvmVal::obj("com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionPacket")]);
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "text", vec![JvmVal::string("")]).call("sendStatus", vec![p2.clone()]);
                // putstatic: mod-own statics are modeled at module scope

                // putstatic: mod-own statics are modeled at module scope

                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler.class :: fail (java static)
pub fn fail(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("player");
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![p0.clone(), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("sendStatus", vec![arg_tmp_0.clone()]);
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/tool/CardboardSwordItem.class :: supportsEnchantment (java instance)
pub fn supports_enchantment(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_ref_eq(&p1.call("getKey", vec![]), &ctx.call_static("net/minecraft/world/item/enchantment/Enchantments", "KNOCKBACK", vec![])) { pc = 16; continue 'jvm; }
                pc = 13;
            }
            12 => {
                pc = 17;
            }
            16 => {
                pc = 17;
            }
            17 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/tool/CardboardSwordItem.class :: isBookEnchantable (java instance)
pub fn is_book_enchantable(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/item/enchantment/ItemEnchantments", "EMPTY", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/core/component/DataComponents", "STORED_ENCHANTMENTS", vec![]);
                local_3 = (p1.call("getOrDefault", vec![arg_tmp_1.clone(), arg_tmp_0.clone()])).clone();
                local_4 = (local_3.call("keySet", vec![]).call("iterator", vec![])).clone();
                pc = 27;
            }
            25 => {
                if jvm_cmp(&local_4.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 65; continue 'jvm; }
                pc = 37;
            }
            35 => {
                local_5 = (local_4.call("next", vec![])).clone();
                if jvm_ref_eq(&local_5.call("getKey", vec![]), &ctx.call_static("net/minecraft/world/item/enchantment/Enchantments", "KNOCKBACK", vec![])) { pc = 62; continue 'jvm; }
                pc = 61;
            }
            60 => {
                return JvmVal::Int(0);
            }
            62 => {
                pc = 25;
            }
            65 => {
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/tool/CardboardSwordItem.class :: cardboardSwordsMakeNoiseOnClick (java static)
pub fn cardboard_swords_make_noise_on_click(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getItemStack", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "CARDBOARD_SWORD", vec![]).call("isIn", vec![local_1.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 16; continue 'jvm; }
                pc = 16;
            }
            15 => {
                return;
            }
            16 => {
                if jvm_ref_eq(&p0.call("getAction", vec![]), &ctx.call_static("net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$LeftClickBlock$Action", "START", vec![])) { pc = 27; continue 'jvm; }
                pc = 27;
            }
            26 => {
                return;
            }
            27 => {
                if !jvm_ref_eq(&p0.call("getSide", vec![]), &ctx.call_static("net/neoforged/fml/LogicalSide", "CLIENT", vec![])) { pc = 59; continue 'jvm; }
                pc = 40;
            }
            37 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "CARDBOARD_SWORD", vec![]).call("playAt", vec![p0.call("getLevel", vec![]), p0.call("getPos", vec![]), JvmVal::Float(0.5_f32), JvmVal::Float(1.85_f32), JvmVal::Int(0)]);
                pc = 81;
            }
            59 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "CARDBOARD_SWORD", vec![]).call("play", vec![p0.call("getLevel", vec![]), p0.call("getEntity", vec![]), p0.call("getPos", vec![]), JvmVal::Float(0.5_f32), JvmVal::Float(1.85_f32)]);
                pc = 81;
            }
            81 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/tool/CardboardSwordItem.class :: cardboardSwordsCannotHurtYou (java static)
pub fn cardboard_swords_cannot_hurt_you(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getEntity", vec![])).clone();
                local_3 = (p0.call("getTarget", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_3, "net/minecraft/world/entity/LivingEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 35; continue 'jvm; }
                pc = 18;
            }
            17 => {
                local_2 = (local_3).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/tags/EntityTypeTags", "ARTHROPOD", vec![]);
                if jvm_cmp(&local_2.call("getType", vec![]).call("is", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 36; continue 'jvm; }
                pc = 36;
            }
            35 => {
                return;
            }
            36 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionHand", "MAIN_HAND", vec![]);
                local_3 = (local_1.call("getItemInHand", vec![arg_tmp_0.clone()])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "CARDBOARD_SWORD", vec![]).call("isIn", vec![local_3.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 55; continue 'jvm; }
                pc = 55;
            }
            54 => {
                return;
            }
            55 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "CARDBOARD_SWORD", vec![]).call("playFrom", vec![local_1.clone(), JvmVal::Float(0.75_f32), JvmVal::Float(1.85_f32)]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/entity/ai/attributes/Attributes", "ATTACK_KNOCKBACK", vec![]);
                local_4 = (jvm_conv(&jvm_bin("+", &local_1.call("getAttributeValue", vec![arg_tmp_0.clone()]), &JvmVal::Double(2_f64)), "F")).clone();
                local_6 = (local_1.call("level", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_6, "net/minecraft/server/level/ServerLevel"), &JvmVal::Int(0)).as_i32() == 0 { pc = 126; continue 'jvm; }
                pc = 101;
            }
            99 => {
                local_5 = (local_6).clone();
                local_4 = (ctx.call_static("net/minecraft/world/item/enchantment/EnchantmentHelper", "modifyKnockback", vec![local_5.clone(), local_3.clone(), local_2.clone(), local_5.call("damageSources", vec![]).call("playerAttack", vec![local_1.clone()]), local_4.clone()])).clone();
                pc = 127;
            }
            126 => {
                if jvm_cmp(&local_1.call("isSprinting", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 151; continue 'jvm; }
                pc = 134;
            }
            133 => {
                if jvm_cmp(&jvm_cmp(&local_1.call("getAttackStrengthScale", vec![JvmVal::Float(0.5_f32)]), &JvmVal::Float(0.9_f32)), &JvmVal::Int(0)).as_i32() <= 0 { pc = 151; continue 'jvm; }
                pc = 147;
            }
            145 => {
                local_4 = (jvm_bin("+", &local_4, &JvmVal::Float(1_f32))).clone();
                pc = 153;
            }
            151 => {
                if jvm_cmp(&jvm_cmp(&local_4, &JvmVal::Float(0_f32)), &JvmVal::Int(0)).as_i32() > 0 { pc = 159; continue 'jvm; }
                pc = 159;
            }
            158 => {
                return;
            }
            159 => {
                local_5 = (local_1.call("getYRot", vec![])).clone();
                ctx.call_static("com/simibubi/create/content/equipment/tool/CardboardSwordItem", "knockback", vec![local_2.clone(), jvm_conv(&local_4, "D"), local_5.clone()]);
                local_6 = (jvm_instanceof(&local_2, "net/minecraft/world/entity/player/Player")).clone();
                local_7 = (local_2.call("getClassification", vec![JvmVal::Int(0)])).clone();
                if jvm_cmp(&jvm_instanceof(&local_2, "net/minecraft/server/level/ServerPlayer"), &JvmVal::Int(0)).as_i32() == 0 { pc = 221; continue 'jvm; }
                pc = 195;
            }
            194 => {
                local_8 = (local_2).clone();
                local_8.call("sendToClient", vec![local_8.clone(), JvmVal::obj("com/simibubi/create/content/equipment/tool/KnockbackPacket")]);
                pc = 223;
            }
            221 => {
                if jvm_ref_eq(&local_7, &ctx.call_static("net/minecraft/world/entity/MobCategory", "MISC", vec![])) { pc = 237; continue 'jvm; }
                pc = 231;
            }
            229 => {
                if !jvm_ref_eq(&local_7, &ctx.call_static("net/minecraft/world/entity/MobCategory", "CREATURE", vec![])) { pc = 264; continue 'jvm; }
                pc = 239;
            }
            237 => {
                if jvm_cmp(&local_6, &JvmVal::Int(0)).as_i32() != 0 { pc = 264; continue 'jvm; }
                pc = 243;
            }
            242 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/effect/MobEffects", "MOVEMENT_SLOWDOWN", vec![]);
                pc = 265;
            }
            264 => {
                local_1.call("setDeltaMovement", vec![local_1.call("getDeltaMovement", vec![]).call("multiply", vec![JvmVal::Double(0.6_f64), JvmVal::Double(1_f64), JvmVal::Double(0.6_f64)])]);
                local_1.call("setSprinting", vec![JvmVal::Int(0)]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/tool/CardboardSwordItem.class :: knockback (java static)
pub fn knockback(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                p0.call("stopRiding", vec![]);
    let arg_tmp_0 = jvm_conv(&jvm_neg(&ctx.call_static("net/minecraft/util/Mth", "cos", vec![jvm_bin("*", &p2, &JvmVal::Float(0.017453292_f32))])), "D");
    let arg_tmp_1 = jvm_conv(&ctx.call_static("net/minecraft/util/Mth", "sin", vec![jvm_bin("*", &p2, &JvmVal::Float(0.017453292_f32))]), "D");
                p0.call("knockback", vec![jvm_bin("*", &p1, &JvmVal::Double(0.5_f64)), arg_tmp_1.clone(), arg_tmp_0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/tool/CardboardSwordItem.class :: initializeClient (java instance)
pub fn initialize_client(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/item/render/SimpleCustomRenderer", "create", vec![p_this.clone(), JvmVal::obj("com/simibubi/create/content/equipment/tool/CardboardSwordItemRenderer")]);
                p_this.call("accept", vec![arg_tmp_0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/zapper/ZapperInteractionHandler.class :: leftClickingBlocksWithTheZapperSelectsTheBlock (java static)
pub fn left_clicking_blocks_with_the_zapper_selects_the_block(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p0.call("getLevel", vec![]).get_field("isClientSide"), &JvmVal::Int(0)).as_i32() == 0 { pc = 11; continue 'jvm; }
                pc = 11;
            }
            10 => {
                return;
            }
            11 => {
                local_1 = (p0.call("getEntity", vec![]).call("getMainHandItem", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_1.call("getItem", vec![]), "com/simibubi/create/content/equipment/zapper/ZapperItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 45; continue 'jvm; }
                pc = 30;
            }
            29 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/zapper/ZapperInteractionHandler", "trySelect", vec![local_1.clone(), p0.call("getEntity", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 45; continue 'jvm; }
                pc = 41;
            }
            40 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                pc = 45;
            }
            45 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/zapper/ZapperInteractionHandler.class :: trySelect (java static)
pub fn try_select(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p1.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 9; continue 'jvm; }
                pc = 8;
            }
            7 => {
                return JvmVal::Int(0);
            }
            9 => {
                local_2 = (p1.call("position", vec![]).call("add", vec![JvmVal::Double(0_f64), jvm_conv(&p1.call("getEyeHeight", vec![]), "D"), JvmVal::Double(0_f64)])).clone();
    let arg_tmp_0 = jvm_conv(&ctx.call_static("com/simibubi/create/content/equipment/zapper/ZapperInteractionHandler", "getRange", vec![p0.clone()]), "D");
                local_3 = (p1.call("getLookAngle", vec![]).call("scale", vec![arg_tmp_0.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/level/ClipContext$Fluid", "NONE", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/world/level/ClipContext$Block", "OUTLINE", vec![]);
                local_4 = (p1.call("level", vec![]).call("clip", vec![JvmVal::obj("net/minecraft/world/level/ClipContext")])).clone();
                local_5 = (local_4.call("getBlockPos", vec![])).clone();
                if !jvm_is_null(&local_5) { pc = 80; continue 'jvm; }
                pc = 79;
            }
            78 => {
                return JvmVal::Int(0);
            }
            80 => {
                p1.call("level", vec![]).call("destroyBlockProgress", vec![p1.call("getId", vec![]), local_5.clone(), JvmVal::Int(-1)]);
                local_6 = (p1.call("level", vec![]).call("getBlockState", vec![local_5.clone()])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/foundation/utility/BlockHelper", "getRequiredItem", vec![local_6.clone()]).call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 118; continue 'jvm; }
                pc = 117;
            }
            116 => {
                return JvmVal::Int(0);
            }
            118 => {
                if jvm_cmp(&local_6.call("hasBlockEntity", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 139; continue 'jvm; }
                pc = 129;
            }
            126 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllTags$AllBlockTags", "SAFE_NBT", vec![]).call("matches", vec![local_6.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 139; continue 'jvm; }
                pc = 138;
            }
            137 => {
                return JvmVal::Int(0);
            }
            139 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/level/block/state/properties/BlockStateProperties", "DOUBLE_BLOCK_HALF", vec![]);
                if jvm_cmp(&local_6.call("hasProperty", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 152; continue 'jvm; }
                pc = 151;
            }
            150 => {
                return JvmVal::Int(0);
            }
            152 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/level/block/state/properties/BlockStateProperties", "ATTACHED", vec![]);
                if jvm_cmp(&local_6.call("hasProperty", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 165; continue 'jvm; }
                pc = 164;
            }
            163 => {
                return JvmVal::Int(0);
            }
            165 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/level/block/state/properties/BlockStateProperties", "HANGING", vec![]);
                if jvm_cmp(&local_6.call("hasProperty", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 178; continue 'jvm; }
                pc = 177;
            }
            176 => {
                return JvmVal::Int(0);
            }
            178 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/level/block/state/properties/BlockStateProperties", "BED_PART", vec![]);
                if jvm_cmp(&local_6.call("hasProperty", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 191; continue 'jvm; }
                pc = 190;
            }
            189 => {
                return JvmVal::Int(0);
            }
            191 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/level/block/state/properties/BlockStateProperties", "STAIRS_SHAPE", vec![]);
                if jvm_cmp(&local_6.call("hasProperty", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 218; continue 'jvm; }
                pc = 204;
            }
            202 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/level/block/state/properties/StairsShape", "STRAIGHT", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/world/level/block/state/properties/BlockStateProperties", "STAIRS_SHAPE", vec![]);
                local_6 = (local_6.call("setValue", vec![arg_tmp_1.clone(), arg_tmp_0.clone()])).clone();
                pc = 220;
            }
            218 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/level/block/state/properties/BlockStateProperties", "PERSISTENT", vec![]);
                if jvm_cmp(&local_6.call("hasProperty", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 246; continue 'jvm; }
                pc = 231;
            }
            229 => {
    let arg_tmp_0 = ctx.call_static("java/lang/Boolean", "valueOf", vec![JvmVal::Int(1)]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/world/level/block/state/properties/BlockStateProperties", "PERSISTENT", vec![]);
                local_6 = (local_6.call("setValue", vec![arg_tmp_1.clone(), arg_tmp_0.clone()])).clone();
                pc = 248;
            }
            246 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/level/block/state/properties/BlockStateProperties", "WATERLOGGED", vec![]);
                if jvm_cmp(&local_6.call("hasProperty", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 274; continue 'jvm; }
                pc = 259;
            }
            257 => {
    let arg_tmp_0 = ctx.call_static("java/lang/Boolean", "valueOf", vec![JvmVal::Int(0)]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/world/level/block/state/properties/BlockStateProperties", "WATERLOGGED", vec![]);
                local_6 = (local_6.call("setValue", vec![arg_tmp_1.clone(), arg_tmp_0.clone()])).clone();
                pc = 275;
            }
            274 => {
                local_7 = (JvmVal::Null).clone();
                local_8 = (p1.call("level", vec![]).call("getBlockEntity", vec![local_5.clone()])).clone();
                if jvm_is_null(&local_8) { pc = 332; continue 'jvm; }
                pc = 295;
            }
            293 => {
                local_7 = (local_8.call("saveWithFullMetadata", vec![p1.call("registryAccess", vec![])])).clone();
                local_7.call("remove", vec![JvmVal::string("x")]);
                local_7.call("remove", vec![JvmVal::string("y")]);
                local_7.call("remove", vec![JvmVal::string("z")]);
                local_7.call("remove", vec![JvmVal::string("id")]);
                pc = 333;
            }
            332 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "SHAPER_BLOCK_USED", vec![]);
                if jvm_cmp(&p0.call("has", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 371; continue 'jvm; }
                pc = 343;
            }
            342 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "SHAPER_BLOCK_USED", vec![]);
                if !jvm_ref_eq(&p0.call("get", vec![arg_tmp_0.clone()]), &local_6) { pc = 371; continue 'jvm; }
                pc = 356;
            }
            354 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "SHAPER_BLOCK_DATA", vec![]);
                if jvm_cmp(&ctx.call_static("java/util/Objects", "equals", vec![local_7.clone(), p0.call("get", vec![arg_tmp_0.clone()])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 371; continue 'jvm; }
                pc = 370;
            }
            369 => {
                return JvmVal::Int(0);
            }
            371 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "SHAPER_BLOCK_USED", vec![]);
                if !jvm_is_null(&local_7) { pc = 397; continue 'jvm; }
                pc = 387;
            }
            386 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "SHAPER_BLOCK_DATA", vec![]);
                pc = 407;
            }
            397 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "SHAPER_BLOCK_DATA", vec![]);
                pc = 410;
            }
            407 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "CONFIRM", vec![]).call("playOnServer", vec![p1.call("level", vec![]), p1.call("blockPosition", vec![])]);
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/zapper/ZapperInteractionHandler.class :: getRange (java static)
pub fn get_range(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p0.call("getItem", vec![]), "com/simibubi/create/content/equipment/zapper/ZapperItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 22; continue 'jvm; }
                pc = 11;
            }
            10 => {
                return p0.call("getItem", vec![]).call("getZappingRange", vec![p0.clone()]);
            }
            22 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/symmetryWand/SymmetryHandler.class :: onBlockPlaced (java static)
pub fn on_block_placed(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p0.call("getLevel", vec![]).call("isClientSide", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 13; continue 'jvm; }
                pc = 13;
            }
            12 => {
                return;
            }
            13 => {
                local_2 = (p0.call("getEntity", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_2, "net/minecraft/world/entity/player/Player"), &JvmVal::Int(0)).as_i32() == 0 { pc = 33; continue 'jvm; }
                pc = 26;
            }
            25 => {
                local_1 = (local_2).clone();
                pc = 34;
            }
            33 => {
                return;
            }
            34 => {
                local_2 = (local_1.call("getInventory", vec![])).clone();
                local_3 = (JvmVal::Int(0)).clone();
                pc = 42;
            }
            41 => {
                if jvm_cmp(&local_3, &ctx.call_static("net/minecraft/world/entity/player/Inventory", "getSelectionSize", vec![])).as_i32() >= 0 { pc = 89; continue 'jvm; }
                pc = 51;
            }
            48 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WAND_OF_SYMMETRY", vec![]).call("isIn", vec![local_2.call("getItem", vec![local_3.clone()])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 83; continue 'jvm; }
                pc = 63;
            }
            62 => {
                ctx.call_static("com/simibubi/create/content/equipment/symmetryWand/SymmetryWandItem", "apply", vec![local_1.call("level", vec![]), local_2.call("getItem", vec![local_3.clone()]), local_1.clone(), p0.call("getPos", vec![]), p0.call("getPlacedBlock", vec![])]);
                pc = 86;
            }
            83 => {
                local_3 = jvm_iinc(&local_3, 1);
                pc = 41;
            }
            89 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/symmetryWand/SymmetryHandler.class :: onBlockDestroyed (java static)
pub fn on_block_destroyed(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p0.call("getLevel", vec![]).call("isClientSide", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 13; continue 'jvm; }
                pc = 13;
            }
            12 => {
                return;
            }
            13 => {
                local_1 = (p0.call("getPlayer", vec![])).clone();
                local_2 = (local_1.call("getInventory", vec![])).clone();
                local_3 = (JvmVal::Int(0)).clone();
                pc = 26;
            }
            25 => {
                if jvm_cmp(&local_3, &ctx.call_static("net/minecraft/world/entity/player/Inventory", "getSelectionSize", vec![])).as_i32() >= 0 { pc = 69; continue 'jvm; }
                pc = 35;
            }
            32 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WAND_OF_SYMMETRY", vec![]).call("isIn", vec![local_2.call("getItem", vec![local_3.clone()])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 63; continue 'jvm; }
                pc = 47;
            }
            46 => {
                ctx.call_static("com/simibubi/create/content/equipment/symmetryWand/SymmetryWandItem", "remove", vec![local_1.call("level", vec![]), local_2.call("getItem", vec![local_3.clone()]), local_1.clone(), p0.call("getPos", vec![])]);
                pc = 66;
            }
            63 => {
                local_3 = jvm_iinc(&local_3, 1);
                pc = 25;
            }
            69 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/symmetryWand/SymmetryHandler.class :: onRenderWorld (java static)
pub fn on_render_world(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut local_14: JvmVal = JvmVal::Null;
    let mut local_15: JvmVal = JvmVal::Null;
    let mut local_16: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_ref_eq(&p0.call("getStage", vec![]), &ctx.call_static("net/neoforged/neoforge/client/event/RenderLevelStageEvent$Stage", "AFTER_PARTICLES", vec![])) { pc = 11; continue 'jvm; }
                pc = 11;
            }
            10 => {
                return;
            }
            11 => {
                local_1 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![])).clone();
                local_2 = (local_1.get_field("player")).clone();
                local_3 = (ctx.call_static("net/minecraft/util/RandomSource", "create", vec![])).clone();
                local_4 = (JvmVal::Int(0)).clone();
                pc = 29;
            }
            27 => {
                if jvm_cmp(&local_4, &ctx.call_static("net/minecraft/world/entity/player/Inventory", "getSelectionSize", vec![])).as_i32() >= 0 { pc = 299; continue 'jvm; }
                pc = 36;
            }
            35 => {
                local_5 = (local_2.call("getInventory", vec![]).call("getItem", vec![local_4.clone()])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WAND_OF_SYMMETRY", vec![]).call("isIn", vec![local_5.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 60; continue 'jvm; }
                pc = 60;
            }
            57 => {
                pc = 293;
            }
            60 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/symmetryWand/SymmetryWandItem", "isEnabled", vec![local_5.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 71; continue 'jvm; }
                pc = 71;
            }
            68 => {
                pc = 293;
            }
            71 => {
                local_6 = (ctx.call_static("com/simibubi/create/content/equipment/symmetryWand/SymmetryWandItem", "getMirror", vec![local_5.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_6, "com/simibubi/create/content/equipment/symmetryWand/mirror/EmptyMirror"), &JvmVal::Int(0)).as_i32() == 0 { pc = 89; continue 'jvm; }
                pc = 89;
            }
            86 => {
                pc = 293;
            }
            89 => {
                local_7 = (ctx.call_static("net/minecraft/core/BlockPos", "containing", vec![local_6.call("getPosition", vec![])])).clone();
                local_8 = (JvmVal::Float(0_f32)).clone();
                local_9 = (JvmVal::Double(0.0625_f64)).clone();
    let arg_tmp_0 = jvm_conv(&jvm_bin("*", &jvm_conv(&ctx.call_static("net/createmod/catnip/animation/AnimationTickHolder", "getRenderTime", vec![]), "D"), &local_9), "F");
                local_8 = (jvm_bin("/", &ctx.call_static("net/minecraft/util/Mth", "sin", vec![arg_tmp_0.clone()]), &JvmVal::Float(5_f32))).clone();
                local_11 = (local_1.call("renderBuffers", vec![]).call("bufferSource", vec![])).clone();
                local_12 = (local_1.get_field("gameRenderer").call("getMainCamera", vec![])).clone();
                local_13 = (local_12.call("getPosition", vec![])).clone();
                local_14 = (p0.call("getPoseStack", vec![])).clone();
                local_14.call("pushPose", vec![]);
                local_14.call("translate", vec![jvm_bin("-", &jvm_conv(&local_7.call("getX", vec![]), "D"), &local_13.call("x", vec![])), jvm_bin("-", &jvm_conv(&local_7.call("getY", vec![]), "D"), &local_13.call("y", vec![])), jvm_bin("-", &jvm_conv(&local_7.call("getZ", vec![]), "D"), &local_13.call("z", vec![]))]);
                local_14.call("translate", vec![JvmVal::Float(0_f32), jvm_bin("+", &local_8, &JvmVal::Float(0.2_f32)), JvmVal::Float(0_f32)]);
                local_6.call("applyModelTransform", vec![local_14.clone()]);
                local_15 = (local_6.call("getModel", vec![]).call("get", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/client/renderer/RenderType", "solid", vec![]);
                local_16 = (local_11.call("getBuffer", vec![arg_tmp_0.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/client/renderer/RenderType", "solid", vec![]);
    let arg_tmp_1 = ctx.call_static("net/neoforged/neoforge/client/model/data/ModelData", "EMPTY", vec![]);
    let arg_tmp_2 = ctx.call_static("net/minecraft/client/renderer/texture/OverlayTexture", "NO_OVERLAY", vec![]);
    let arg_tmp_3 = ctx.call_static("net/minecraft/util/Mth", "getSeed", vec![local_7.clone()]);
    let arg_tmp_4 = ctx.call_static("net/minecraft/world/level/block/Blocks", "AIR", vec![]).call("defaultBlockState", vec![]);
                local_1.call("getBlockRenderer", vec![]).call("getModelRenderer", vec![]).call("tesselateBlock", vec![local_2.call("level", vec![]), local_15.clone(), arg_tmp_4.clone(), local_7.clone(), local_14.clone(), local_16.clone(), JvmVal::Int(1), local_3.clone(), arg_tmp_3.clone(), arg_tmp_2.clone(), arg_tmp_1.clone(), arg_tmp_0.clone()]);
                local_14.call("popPose", vec![]);
                local_11.call("endBatch", vec![]);
                pc = 296;
            }
            293 => {
                local_4 = jvm_iinc(&local_4, 1);
                pc = 27;
            }
            299 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/symmetryWand/SymmetryHandler.class :: onClientTick (java static)
pub fn on_client_tick(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![])).clone();
                local_2 = (local_1.get_field("player")).clone();
                if !jvm_is_null(&local_1.get_field("level")) { pc = 17; continue 'jvm; }
                pc = 17;
            }
            16 => {
                return;
            }
            17 => {
                if jvm_cmp(&local_1.call("isPaused", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 25; continue 'jvm; }
                pc = 25;
            }
            24 => {
                return;
            }
            25 => {
                // putstatic: mod-own statics are modeled at module scope

                if jvm_cmp(&jvm_bin("%", &ctx.call_static("com/simibubi/create/content/equipment/symmetryWand/SymmetryHandler", "tickCounter", vec![]), &JvmVal::Int(10)), &JvmVal::Int(0)).as_i32() != 0 { pc = 241; continue 'jvm; }
                pc = 43;
            }
            42 => {
                local_3 = (JvmVal::Int(0)).clone();
                pc = 45;
            }
            44 => {
                if jvm_cmp(&local_3, &ctx.call_static("net/minecraft/world/entity/player/Inventory", "getSelectionSize", vec![])).as_i32() >= 0 { pc = 241; continue 'jvm; }
                pc = 52;
            }
            51 => {
                local_4 = (local_2.call("getInventory", vec![]).call("getItem", vec![local_3.clone()])).clone();
                if jvm_is_null(&local_4) { pc = 235; continue 'jvm; }
                pc = 69;
            }
            66 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WAND_OF_SYMMETRY", vec![]).call("isIn", vec![local_4.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 235; continue 'jvm; }
                pc = 79;
            }
            77 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/symmetryWand/SymmetryWandItem", "isEnabled", vec![local_4.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 235; continue 'jvm; }
                pc = 87;
            }
            85 => {
                local_5 = (ctx.call_static("com/simibubi/create/content/equipment/symmetryWand/SymmetryWandItem", "getMirror", vec![local_4.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_5, "com/simibubi/create/content/equipment/symmetryWand/mirror/EmptyMirror"), &JvmVal::Int(0)).as_i32() == 0 { pc = 103; continue 'jvm; }
                pc = 103;
            }
            100 => {
                pc = 235;
            }
            103 => {
                local_6 = (local_1.get_field("level").get_field("random")).clone();
                local_7 = (jvm_bin("*", &jvm_bin("-", &local_6.call("nextDouble", vec![]), &JvmVal::Double(0.5_f64)), &JvmVal::Double(0.3_f64))).clone();
                local_9 = (jvm_bin("*", &jvm_bin("-", &local_6.call("nextDouble", vec![]), &JvmVal::Double(0.5_f64)), &JvmVal::Double(0.3_f64))).clone();
                local_11 = (local_5.call("getPosition", vec![]).call("add", vec![jvm_bin("+", &JvmVal::Double(0.5_f64), &local_7), JvmVal::Double(0.25_f64), jvm_bin("+", &JvmVal::Double(0.5_f64), &local_9)])).clone();
                local_12 = (JvmVal::obj("net/minecraft/world/phys/Vec3")).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/particles/ParticleTypes", "END_ROD", vec![]);
                local_1.get_field("level").call("addParticle", vec![arg_tmp_0.clone(), local_11.get_field("x"), local_11.get_field("y"), local_11.get_field("z"), local_12.get_field("x"), local_12.get_field("y"), local_12.get_field("z")]);
                pc = 238;
            }
            235 => {
                local_3 = jvm_iinc(&local_3, 1);
                pc = 44;
            }
            241 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/symmetryWand/SymmetryHandler.class :: drawEffect (java static)
pub fn draw_effect(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("level")).clone();
                local_3 = (local_2.get_field("random")).clone();
                local_4 = (JvmVal::Double(0.800000011920929_f64)).clone();
                local_6 = (ctx.call_static("net/minecraft/world/phys/Vec3", "atLowerCornerOf", vec![p0.clone()]).call("add", vec![JvmVal::Double(0.5_f64), JvmVal::Double(0.5_f64), JvmVal::Double(0.5_f64)])).clone();
                local_7 = (ctx.call_static("net/minecraft/world/phys/Vec3", "atLowerCornerOf", vec![p1.clone()]).call("add", vec![JvmVal::Double(0.5_f64), JvmVal::Double(0.5_f64), JvmVal::Double(0.5_f64)])).clone();
                local_8 = (local_7.call("subtract", vec![local_6.clone()])).clone();
                local_9 = (local_8.call("normalize", vec![]).call("scale", vec![local_4.clone()])).clone();
                local_10 = (jvm_conv(&jvm_bin("/", &local_8.call("length", vec![]), &local_9.call("length", vec![])), "I")).clone();
                local_11 = (JvmVal::Int(3)).clone();
                pc = 93;
            }
            91 => {
                if jvm_cmp(&local_11, &jvm_bin("-", &local_10, &JvmVal::Int(1))).as_i32() >= 0 { pc = 194; continue 'jvm; }
                pc = 102;
            }
            100 => {
                local_12 = (local_6.call("add", vec![local_9.call("scale", vec![jvm_conv(&local_11, "D")])])).clone();
                local_13 = (JvmVal::obj("net/minecraft/world/phys/Vec3")).clone();
                local_2.call("addParticle", vec![JvmVal::obj("net/minecraft/core/particles/DustParticleOptions"), local_12.get_field("x"), local_12.get_field("y"), local_12.get_field("z"), local_13.get_field("x"), local_13.get_field("y"), local_13.get_field("z")]);
                local_11 = jvm_iinc(&local_11, 1);
                pc = 91;
            }
            194 => {
                local_11 = (JvmVal::obj("net/minecraft/world/phys/Vec3")).clone();
                local_12 = (local_6.call("add", vec![local_9.call("scale", vec![JvmVal::Double(2_f64)])])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/particles/ParticleTypes", "END_ROD", vec![]);
                local_2.call("addParticle", vec![arg_tmp_0.clone(), local_12.get_field("x"), local_12.get_field("y"), local_12.get_field("z"), local_11.get_field("x"), local_11.get_field("y"), local_11.get_field("z")]);
                local_11 = (JvmVal::obj("net/minecraft/world/phys/Vec3")).clone();
                local_12 = (local_6.call("add", vec![local_9.call("scale", vec![jvm_conv(&local_10, "D")])])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/particles/ParticleTypes", "END_ROD", vec![]);
                local_2.call("addParticle", vec![arg_tmp_0.clone(), local_12.get_field("x"), local_12.get_field("y"), local_12.get_field("z"), local_11.get_field("x"), local_11.get_field("y"), local_11.get_field("z")]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/wrench/WrenchEventHandler.class :: useOwnWrenchLogicForCreateBlocks (java static)
pub fn use_own_wrench_logic_for_create_blocks(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getEntity", vec![])).clone();
                local_2 = (p0.call("getItemStack", vec![])).clone();
                if jvm_cmp(&p0.call("isCanceled", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 18; continue 'jvm; }
                pc = 18;
            }
            17 => {
                return;
            }
            18 => {
                if !jvm_is_null(&p0.call("getLevel", vec![])) { pc = 26; continue 'jvm; }
                pc = 26;
            }
            25 => {
                return;
            }
            26 => {
                if jvm_is_null(&local_1) { pc = 37; continue 'jvm; }
                pc = 31;
            }
            30 => {
                if jvm_cmp(&local_1.call("mayBuild", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 38; continue 'jvm; }
                pc = 38;
            }
            37 => {
                return;
            }
            38 => {
                if jvm_cmp(&local_2.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 46; continue 'jvm; }
                pc = 46;
            }
            45 => {
                return;
            }
            46 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WRENCH", vec![]).call("isIn", vec![local_2.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 57; continue 'jvm; }
                pc = 57;
            }
            56 => {
                return;
            }
            57 => {
    let arg_tmp_0 = ctx.call_static("net/neoforged/neoforge/common/Tags$Items", "TOOLS_WRENCH", vec![]);
                if jvm_cmp(&local_2.call("is", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 68; continue 'jvm; }
                pc = 68;
            }
            67 => {
                return;
            }
            68 => {
                local_3 = (p0.call("getLevel", vec![]).call("getBlockState", vec![p0.call("getPos", vec![])])).clone();
                local_4 = (local_3.call("getBlock", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_4, "com/simibubi/create/content/equipment/wrench/IWrenchable"), &JvmVal::Int(0)).as_i32() == 0 { pc = 104; continue 'jvm; }
                pc = 96;
            }
            94 => {
                local_5 = (local_4).clone();
                pc = 105;
            }
            104 => {
                return;
            }
            105 => {
                local_6 = (p0.call("getHitVec", vec![])).clone();
                local_7 = (JvmVal::obj("net/minecraft/world/item/context/UseOnContext")).clone();
                if jvm_cmp(&local_1.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 147; continue 'jvm; }
                pc = 136;
            }
            134 => {
                pc = 157;
            }
            147 => {
                pc = 159;
            }
            157 => {
                local_8 = (local_5.call("onWrenched", vec![local_3.clone(), local_7.clone()])).clone();
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                p0.call("setCancellationResult", vec![local_8.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler.class :: drawCustomBlockSelection (java static)
pub fn draw_custom_block_selection(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![])).clone();
                local_2 = (p0.call("getTarget", vec![])).clone();
                local_3 = (local_2.call("getBlockPos", vec![])).clone();
                local_4 = (local_1.get_field("level").call("getBlockState", vec![local_3.clone()])).clone();
                if jvm_is_null(&local_1.get_field("player")) { pc = 41; continue 'jvm; }
                pc = 32;
            }
            31 => {
                if jvm_cmp(&local_1.get_field("player").call("isSpectator", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 42; continue 'jvm; }
                pc = 42;
            }
            41 => {
                return;
            }
            42 => {
                if jvm_cmp(&local_1.get_field("level").call("getWorldBorder", vec![]).call("isWithinBounds", vec![local_3.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 57; continue 'jvm; }
                pc = 57;
            }
            56 => {
                return;
            }
            57 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "CLIPBOARD", vec![]).call("isIn", vec![local_1.get_field("player").call("getMainHandItem", vec![])]), &JvmVal::Int(0)).as_i32() != 0 { pc = 74; continue 'jvm; }
                pc = 74;
            }
            73 => {
                return;
            }
            74 => {
                local_6 = (local_1.get_field("level").call("getBlockEntity", vec![local_3.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_6, "com/simibubi/create/foundation/blockEntity/SmartBlockEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 102; continue 'jvm; }
                pc = 94;
            }
            92 => {
                local_5 = (local_6).clone();
                pc = 103;
            }
            102 => {
                return;
            }
            103 => {
                if jvm_cmp(&jvm_instanceof(&local_5, "com/simibubi/create/content/equipment/clipboard/ClipboardBlockEntity"), &JvmVal::Int(0)).as_i32() != 0 { pc = 145; continue 'jvm; }
                pc = 113;
            }
            111 => {
                if jvm_cmp(&local_2.call("noneMatch", vec![JvmVal::Null]), &JvmVal::Int(0)).as_i32() == 0 { pc = 145; continue 'jvm; }
                pc = 138;
            }
            136 => {
                if jvm_cmp(&jvm_instanceof(&local_5, "com/simibubi/create/content/equipment/clipboard/ClipboardCloneable"), &JvmVal::Int(0)).as_i32() != 0 { pc = 145; continue 'jvm; }
                pc = 145;
            }
            144 => {
                return;
            }
            145 => {
                local_6 = (local_4.call("getShape", vec![local_1.get_field("level"), local_3.clone()])).clone();
                if jvm_cmp(&local_6.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 166; continue 'jvm; }
                pc = 166;
            }
            165 => {
                return;
            }
            166 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/client/renderer/RenderType", "lines", vec![]);
                local_7 = (p0.call("getMultiBufferSource", vec![]).call("getBuffer", vec![arg_tmp_0.clone()])).clone();
                local_8 = (p0.call("getCamera", vec![]).call("getPosition", vec![])).clone();
                local_9 = (p0.call("getPoseStack", vec![])).clone();
                local_9.call("pushPose", vec![]);
                local_9.call("translate", vec![jvm_bin("-", &jvm_conv(&local_3.call("getX", vec![]), "D"), &local_8.get_field("x")), jvm_bin("-", &jvm_conv(&local_3.call("getY", vec![]), "D"), &local_8.get_field("y")), jvm_bin("-", &jvm_conv(&local_3.call("getZ", vec![]), "D"), &local_8.get_field("z"))]);
    let arg_tmp_0 = ctx.call_static("java/lang/Boolean", "valueOf", vec![JvmVal::Int(1)]);
                ctx.call_static("com/simibubi/create/content/trains/track/TrackBlockOutline", "renderShape", vec![local_6.clone(), local_9.clone(), local_7.clone(), arg_tmp_0.clone()]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                local_9.call("popPose", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler.class :: clientTick (java static)
pub fn client_tick_2(ctx: &mut BridgeCtx) {
    let mut local_0: JvmVal = JvmVal::Null;
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_0 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![])).clone();
                local_2 = (local_0.get_field("hitResult")).clone();
                if jvm_cmp(&jvm_instanceof(&local_2, "net/minecraft/world/phys/BlockHitResult"), &JvmVal::Int(0)).as_i32() == 0 { pc = 24; continue 'jvm; }
                pc = 17;
            }
            16 => {
                local_1 = (local_2).clone();
                pc = 25;
            }
            24 => {
                return;
            }
            25 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "CLIPBOARD", vec![]).call("isIn", vec![local_0.get_field("player").call("getMainHandItem", vec![])]), &JvmVal::Int(0)).as_i32() != 0 { pc = 42; continue 'jvm; }
                pc = 42;
            }
            41 => {
                return;
            }
            42 => {
                local_2 = (local_1.call("getBlockPos", vec![])).clone();
                local_4 = (local_0.get_field("level").call("getBlockEntity", vec![local_2.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_4, "com/simibubi/create/foundation/blockEntity/SmartBlockEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 74; continue 'jvm; }
                pc = 67;
            }
            65 => {
                local_3 = (local_4).clone();
                pc = 75;
            }
            74 => {
                return;
            }
            75 => {
                if jvm_cmp(&jvm_instanceof(&local_3, "com/simibubi/create/content/equipment/clipboard/ClipboardBlockEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 142; continue 'jvm; }
                pc = 85;
            }
            82 => {
                local_4 = (JvmVal::obj("java/util/ArrayList")).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("clipboard.actions"), jvm_array_new(0)]);
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), ctx.call_static("net/minecraft/network/chat/Component", "keybind", vec![JvmVal::string("key.use")]));
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("clipboard.copy_other_clipboard"), jvm_array_new(0)]);
                ctx.call_static("com/simibubi/create/CreateClient", "VALUE_SETTINGS_HANDLER", vec![]).call("showHoverTip", vec![local_4.clone()]);
                return;
            }
            142 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLIPBOARD_CONTENT", vec![]);
                local_4 = (local_0.get_field("player").call("getMainHandItem", vec![]).call("get", vec![arg_tmp_0.clone()])).clone();
                if !jvm_is_null(&local_4) { pc = 166; continue 'jvm; }
                pc = 166;
            }
            165 => {
                return;
            }
            166 => {
                local_5 = (local_4.call("copiedValues", vec![]).call("orElse", vec![JvmVal::Null])).clone();
                if jvm_cmp(&local_1.call("anyMatch", vec![JvmVal::Null]), &JvmVal::Int(0)).as_i32() != 0 { pc = 245; continue 'jvm; }
                pc = 205;
            }
            204 => {
                if jvm_cmp(&jvm_instanceof(&local_3, "com/simibubi/create/content/equipment/clipboard/ClipboardCloneable"), &JvmVal::Int(0)).as_i32() == 0 { pc = 249; continue 'jvm; }
                pc = 212;
            }
            211 => {
                local_7 = (local_3).clone();
                if jvm_cmp(&local_0.get_field("level").call("registryAccess", vec![]).call("writeToClipboard", vec![local_0.get_field("level").call("registryAccess", vec![]), JvmVal::obj("net/minecraft/nbt/CompoundTag"), local_1.call("getDirection", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 249; continue 'jvm; }
                pc = 246;
            }
            245 => {
                pc = 250;
            }
            249 => {
                pc = 252;
            }
            250 => {
                local_6 = (JvmVal::Int(0)).clone();
                if jvm_is_null(&local_5) { pc = 338; continue 'jvm; }
                pc = 258;
            }
            257 => {
                if jvm_cmp(&local_1.call("anyMatch", vec![JvmVal::Null]), &JvmVal::Int(0)).as_i32() != 0 { pc = 334; continue 'jvm; }
                pc = 284;
            }
            283 => {
                if jvm_cmp(&jvm_instanceof(&local_3, "com/simibubi/create/content/equipment/clipboard/ClipboardCloneable"), &JvmVal::Int(0)).as_i32() == 0 { pc = 338; continue 'jvm; }
                pc = 291;
            }
            290 => {
                local_8 = (local_3).clone();
                if jvm_cmp(&local_8.call("readFromClipboard", vec![local_0.get_field("level").call("registryAccess", vec![]), local_5.call("getCompound", vec![local_8.call("getClipboardKey", vec![])]), local_0.get_field("player"), local_1.call("getDirection", vec![]), JvmVal::Int(1)]), &JvmVal::Int(0)).as_i32() == 0 { pc = 338; continue 'jvm; }
                pc = 335;
            }
            334 => {
                pc = 339;
            }
            338 => {
                pc = 341;
            }
            339 => {
                local_7 = (JvmVal::Int(0)).clone();
                if jvm_cmp(&local_6, &JvmVal::Int(0)).as_i32() != 0 { pc = 352; continue 'jvm; }
                pc = 348;
            }
            346 => {
                if jvm_cmp(&local_7, &JvmVal::Int(0)).as_i32() != 0 { pc = 352; continue 'jvm; }
                pc = 352;
            }
            351 => {
                return;
            }
            352 => {
                local_8 = (JvmVal::obj("java/util/ArrayList")).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("clipboard.actions"), jvm_array_new(0)]);
                if jvm_cmp(&local_6, &JvmVal::Int(0)).as_i32() == 0 { pc = 409; continue 'jvm; }
                pc = 385;
            }
            383 => {
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), ctx.call_static("net/minecraft/network/chat/Component", "keybind", vec![JvmVal::string("key.use")]));
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("clipboard.to_copy"), jvm_array_new(0)]);
                pc = 411;
            }
            409 => {
                if jvm_cmp(&local_7, &JvmVal::Int(0)).as_i32() == 0 { pc = 441; continue 'jvm; }
                pc = 416;
            }
            414 => {
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), ctx.call_static("net/minecraft/network/chat/Component", "keybind", vec![JvmVal::string("key.attack")]));
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("clipboard.to_paste"), jvm_array_new(0)]);
                pc = 444;
            }
            441 => {
                ctx.call_static("com/simibubi/create/CreateClient", "VALUE_SETTINGS_HANDLER", vec![]).call("showHoverTip", vec![local_8.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler.class :: rightClickToCopy (java static)
pub fn right_click_to_copy(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler", "interact", vec![p0.clone(), JvmVal::Int(0)]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler.class :: leftClickToPaste (java static)
pub fn left_click_to_paste(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler", "interact", vec![p0.clone(), JvmVal::Int(1)]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler.class :: interact (java static)
pub fn interact(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut local_14: JvmVal = JvmVal::Null;
    let mut local_15: JvmVal = JvmVal::Null;
    let mut local_16: JvmVal = JvmVal::Null;
    let mut local_17: JvmVal = JvmVal::Null;
    let mut local_18: JvmVal = JvmVal::Null;
    let mut local_19: JvmVal = JvmVal::Null;
    let mut local_20: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (p0.call("getItemStack", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "CLIPBOARD", vec![]).call("isIn", vec![local_2.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 16; continue 'jvm; }
                pc = 16;
            }
            15 => {
                return;
            }
            16 => {
                local_3 = (p0.call("getPos", vec![])).clone();
                local_4 = (p0.call("getLevel", vec![])).clone();
                local_5 = (p0.call("getEntity", vec![])).clone();
                if jvm_is_null(&local_5) { pc = 47; continue 'jvm; }
                pc = 40;
            }
            38 => {
                if jvm_cmp(&local_5.call("isSpectator", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 47; continue 'jvm; }
                pc = 47;
            }
            46 => {
                return;
            }
            47 => {
                if jvm_cmp(&local_5.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 56; continue 'jvm; }
                pc = 56;
            }
            55 => {
                return;
            }
            56 => {
                local_7 = (local_4.call("getBlockEntity", vec![local_3.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_7, "com/simibubi/create/foundation/blockEntity/SmartBlockEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 82; continue 'jvm; }
                pc = 74;
            }
            72 => {
                local_6 = (local_7).clone();
                pc = 83;
            }
            82 => {
                return;
            }
            83 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/clipboard/ClipboardContent", "EMPTY", vec![]);
    let arg_tmp_1 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLIPBOARD_CONTENT", vec![]);
                local_7 = (local_2.call("getOrDefault", vec![arg_tmp_1.clone(), arg_tmp_0.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_6, "com/simibubi/create/content/equipment/clipboard/ClipboardBlockEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 690; continue 'jvm; }
                pc = 108;
            }
            106 => {
                local_8 = (local_6).clone();
                if jvm_cmp(&jvm_instanceof(&p0, "net/neoforged/bus/api/ICancellableEvent"), &JvmVal::Int(0)).as_i32() == 0 { pc = 256; continue 'jvm; }
                pc = 121;
            }
            120 => {
                local_9 = (p0).clone();
                local_9.call("setCanceled", vec![JvmVal::Int(1)]);
                local_10 = (p0).clone();
                local_11 = (JvmVal::Int(0)).clone();
                match JvmVal::Null.as_i32() {
                    0 => { pc = 184; }
                    1 => { pc = 202; }
                    2 => { pc = 220; }
                    3 => { pc = 238; }
                    _ => { pc = 256; }
                }
            }
            184 => {
                local_12 = (local_10).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                local_12.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                pc = 256;
            }
            202 => {
                local_13 = (local_10).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                local_13.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                pc = 256;
            }
            220 => {
                local_14 = (local_10).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                local_14.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                pc = 256;
            }
            238 => {
                local_15 = (local_10).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                local_15.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                pc = 256;
            }
            256 => {
                if jvm_cmp(&local_4.call("isClientSide", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 643; continue 'jvm; }
                pc = 266;
            }
            264 => {
                local_9 = (ctx.call_static("com/simibubi/create/content/equipment/clipboard/ClipboardEntry", "readAll", vec![local_7.clone()])).clone();
                local_10 = (ctx.call_static("com/simibubi/create/content/equipment/clipboard/ClipboardEntry", "readAll", vec![local_8.call("components", vec![])])).clone();
                local_11 = (JvmVal::obj("java/util/ArrayList")).clone();
                local_12 = (local_10.call("iterator", vec![])).clone();
                pc = 301;
            }
            299 => {
                if jvm_cmp(&local_12.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 480; continue 'jvm; }
                pc = 311;
            }
            309 => {
                local_13 = (local_12.call("next", vec![])).clone();
                local_14 = (local_13.call("iterator", vec![])).clone();
                pc = 332;
            }
            330 => {
                if jvm_cmp(&local_14.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 477; continue 'jvm; }
                pc = 342;
            }
            340 => {
                local_15 = (local_14.call("next", vec![])).clone();
                local_16 = (local_15.get_field("text").call("getString", vec![])).clone();
                local_17 = (local_9.call("iterator", vec![])).clone();
                pc = 373;
            }
            371 => {
                if jvm_cmp(&local_17.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 449; continue 'jvm; }
                pc = 383;
            }
            381 => {
                local_18 = (local_17.call("next", vec![])).clone();
                local_19 = (local_18.call("iterator", vec![])).clone();
                pc = 404;
            }
            402 => {
                if jvm_cmp(&local_19.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 446; continue 'jvm; }
                pc = 414;
            }
            412 => {
                local_20 = (local_19.call("next", vec![])).clone();
                if jvm_cmp(&local_16.call("equals", vec![local_20.get_field("text").call("getString", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 443; continue 'jvm; }
                pc = 443;
            }
            440 => {
                pc = 330;
            }
            443 => {
                pc = 402;
            }
            446 => {
                pc = 371;
            }
            449 => {
                pc = 330;
            }
            477 => {
                pc = 299;
            }
            480 => {
                local_12 = (local_11.call("iterator", vec![])).clone();
                pc = 491;
            }
            489 => {
                if jvm_cmp(&local_12.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 624; continue 'jvm; }
                pc = 501;
            }
            499 => {
                local_13 = (local_12.call("next", vec![])).clone();
                local_14 = (JvmVal::Null).clone();
                local_15 = (local_9.call("iterator", vec![])).clone();
                pc = 525;
            }
            523 => {
                if jvm_cmp(&local_15.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 567; continue 'jvm; }
                pc = 535;
            }
            533 => {
                local_16 = (local_15.call("next", vec![])).clone();
                if jvm_cmp(&local_16.call("size", vec![]), &JvmVal::Int(11)).as_i32() <= 0 { pc = 560; continue 'jvm; }
                pc = 560;
            }
            557 => {
                pc = 523;
            }
            560 => {
                local_14 = (local_16).clone();
                pc = 567;
            }
            567 => {
                if !jvm_is_null(&local_14) { pc = 591; continue 'jvm; }
                pc = 575;
            }
            572 => {
                local_14 = (JvmVal::obj("java/util/ArrayList")).clone();
                pc = 593;
            }
            591 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/clipboard/ClipboardOverrides$ClipboardType", "WRITTEN", vec![]);
                local_7 = (local_7.call("setType", vec![arg_tmp_0.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLIPBOARD_CONTENT", vec![]);
                pc = 489;
            }
            624 => {
                local_7 = (local_7.call("setPages", vec![local_9.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLIPBOARD_CONTENT", vec![]);
                pc = 645;
            }
            643 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "WHITE", vec![]);
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), local_4.call("getBlockState", vec![local_3.clone()]).call("getBlock", vec![]).call("getName", vec![]).call("withStyle", vec![arg_tmp_0.clone()]));
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "GREEN", vec![]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("clipboard.copied_from_clipboard"), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("component", vec![]);
                local_5.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                return;
            }
            690 => {
                local_8 = (local_7.call("copiedValues", vec![]).call("orElse", vec![JvmVal::Null])).clone();
                if jvm_cmp(&p1, &JvmVal::Int(0)).as_i32() == 0 { pc = 714; continue 'jvm; }
                pc = 710;
            }
            708 => {
                if !jvm_is_null(&local_8) { pc = 714; continue 'jvm; }
                pc = 714;
            }
            713 => {
                return;
            }
            714 => {
                if jvm_cmp(&p1, &JvmVal::Int(0)).as_i32() != 0 { pc = 727; continue 'jvm; }
                pc = 721;
            }
            718 => {
                local_8 = (JvmVal::obj("net/minecraft/nbt/CompoundTag")).clone();
                pc = 728;
            }
            727 => {
                local_9 = (JvmVal::Int(0)).clone();
                local_10 = (JvmVal::Int(0)).clone();
                local_11 = (local_6.call("getAllBehaviours", vec![]).call("iterator", vec![])).clone();
                pc = 747;
            }
            745 => {
                if jvm_cmp(&local_11.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 890; continue 'jvm; }
                pc = 757;
            }
            755 => {
                local_12 = (local_11.call("next", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_12, "com/simibubi/create/content/equipment/clipboard/ClipboardCloneable"), &JvmVal::Int(0)).as_i32() == 0 { pc = 745; continue 'jvm; }
                pc = 777;
            }
            775 => {
                local_13 = (local_12).clone();
                local_10 = (JvmVal::Int(1)).clone();
                local_14 = (local_13.call("getClipboardKey", vec![])).clone();
                if jvm_cmp(&p1, &JvmVal::Int(0)).as_i32() == 0 { pc = 836; continue 'jvm; }
                pc = 800;
            }
            798 => {
                local_9 = (jvm_bin("|", &local_9, &local_13.call("readFromClipboard", vec![local_4.call("registryAccess", vec![]), local_8.call("getCompound", vec![local_14.clone()]), local_5.clone(), p0.call("getFace", vec![]), local_4.call("isClientSide", vec![])]))).clone();
                pc = 745;
            }
            836 => {
                local_15 = (JvmVal::obj("net/minecraft/nbt/CompoundTag")).clone();
                local_16 = (local_13.call("writeToClipboard", vec![local_4.call("registryAccess", vec![]), local_15.clone(), p0.call("getFace", vec![])])).clone();
                local_9 = (jvm_bin("|", &local_9, &local_16)).clone();
                if jvm_cmp(&local_16, &JvmVal::Int(0)).as_i32() == 0 { pc = 887; continue 'jvm; }
                pc = 879;
            }
            877 => {
                pc = 890;
            }
            887 => {
                pc = 745;
            }
            890 => {
                if jvm_cmp(&jvm_instanceof(&local_6, "com/simibubi/create/content/equipment/clipboard/ClipboardCloneable"), &JvmVal::Int(0)).as_i32() == 0 { pc = 1010; continue 'jvm; }
                pc = 900;
            }
            898 => {
                local_11 = (local_6).clone();
                local_10 = (JvmVal::Int(1)).clone();
                local_12 = (local_11.call("getClipboardKey", vec![])).clone();
                if jvm_cmp(&p1, &JvmVal::Int(0)).as_i32() == 0 { pc = 959; continue 'jvm; }
                pc = 923;
            }
            921 => {
                local_9 = (jvm_bin("|", &local_9, &local_11.call("readFromClipboard", vec![local_4.call("registryAccess", vec![]), local_8.call("getCompound", vec![local_12.clone()]), local_5.clone(), p0.call("getFace", vec![]), local_4.call("isClientSide", vec![])]))).clone();
                pc = 1010;
            }
            959 => {
                local_13 = (JvmVal::obj("net/minecraft/nbt/CompoundTag")).clone();
                local_14 = (local_11.call("writeToClipboard", vec![local_4.call("registryAccess", vec![]), local_13.clone(), p0.call("getFace", vec![])])).clone();
                local_9 = (jvm_bin("|", &local_9, &local_14)).clone();
                if jvm_cmp(&local_14, &JvmVal::Int(0)).as_i32() == 0 { pc = 1010; continue 'jvm; }
                pc = 1002;
            }
            1000 => {
                pc = 1012;
            }
            1010 => {
                if jvm_cmp(&local_10, &JvmVal::Int(0)).as_i32() != 0 { pc = 1016; continue 'jvm; }
                pc = 1016;
            }
            1015 => {
                return;
            }
            1016 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                if jvm_cmp(&jvm_instanceof(&p0, "net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock"), &JvmVal::Int(0)).as_i32() == 0 { pc = 1047; continue 'jvm; }
                pc = 1034;
            }
            1033 => {
                local_11 = (p0).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                local_11.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                pc = 1049;
            }
            1047 => {
                if jvm_cmp(&local_4.call("isClientSide", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 1056; continue 'jvm; }
                pc = 1056;
            }
            1055 => {
                return;
            }
            1056 => {
                if jvm_cmp(&local_9, &JvmVal::Int(0)).as_i32() != 0 { pc = 1062; continue 'jvm; }
                pc = 1062;
            }
            1061 => {
                return;
            }
            1062 => {
                if jvm_cmp(&p1, &JvmVal::Int(0)).as_i32() == 0 { pc = 1074; continue 'jvm; }
                pc = 1071;
            }
            1068 => {
                pc = 1077;
            }
            1074 => {
                pc = 1078;
            }
            1077 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "WHITE", vec![]);
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), local_4.call("getBlockState", vec![local_3.clone()]).call("getBlock", vec![]).call("getName", vec![]).call("withStyle", vec![arg_tmp_0.clone()]));
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "GREEN", vec![]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("clipboard.copied_from"), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("component", vec![]);
                JvmVal::string("clipboard.pasted_to").call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                if jvm_cmp(&p1, &JvmVal::Int(0)).as_i32() != 0 { pc = 1151; continue 'jvm; }
                pc = 1124;
            }
            1122 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/clipboard/ClipboardOverrides$ClipboardType", "WRITTEN", vec![]);
                local_7 = (local_7.call("setType", vec![arg_tmp_0.clone()])).clone();
                local_7 = (local_7.call("setCopiedValues", vec![local_8.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLIPBOARD_CONTENT", vec![]);
                pc = 1151;
            }
            1151 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler.class :: lambda$clientTick$2 (java static)
pub fn lambda_client_tick_2(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p3, "com/simibubi/create/content/equipment/clipboard/ClipboardCloneable"), &JvmVal::Int(0)).as_i32() == 0 { pc = 54; continue 'jvm; }
                pc = 8;
            }
            7 => {
                local_4 = (p3).clone();
                if jvm_cmp(&local_4.call("readFromClipboard", vec![p0.get_field("level").call("registryAccess", vec![]), p1.call("getCompound", vec![local_4.call("getClipboardKey", vec![])]), p0.get_field("player"), p2.call("getDirection", vec![]), JvmVal::Int(1)]), &JvmVal::Int(0)).as_i32() == 0 { pc = 54; continue 'jvm; }
                pc = 51;
            }
            50 => {
                pc = 55;
            }
            54 => {
                pc = 55;
            }
            55 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler.class :: lambda$clientTick$1 (java static)
pub fn lambda_client_tick_1(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p2, "com/simibubi/create/content/equipment/clipboard/ClipboardCloneable"), &JvmVal::Int(0)).as_i32() == 0 { pc = 43; continue 'jvm; }
                pc = 8;
            }
            7 => {
                local_3 = (p2).clone();
                if jvm_cmp(&p0.get_field("level").call("registryAccess", vec![]).call("writeToClipboard", vec![p0.get_field("level").call("registryAccess", vec![]), JvmVal::obj("net/minecraft/nbt/CompoundTag"), p1.call("getDirection", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 43; continue 'jvm; }
                pc = 40;
            }
            39 => {
                pc = 44;
            }
            43 => {
                pc = 44;
            }
            44 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler.class :: lambda$drawCustomBlockSelection$0 (java static)
pub fn lambda_draw_custom_block_selection_0(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p2, "com/simibubi/create/content/equipment/clipboard/ClipboardCloneable"), &JvmVal::Int(0)).as_i32() == 0 { pc = 43; continue 'jvm; }
                pc = 8;
            }
            7 => {
                local_3 = (p2).clone();
                if jvm_cmp(&p0.get_field("level").call("registryAccess", vec![]).call("writeToClipboard", vec![p0.get_field("level").call("registryAccess", vec![]), JvmVal::obj("net/minecraft/nbt/CompoundTag"), p1.call("getDirection", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 43; continue 'jvm; }
                pc = 40;
            }
            39 => {
                pc = 44;
            }
            43 => {
                pc = 44;
            }
            44 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: holdingExtendoGripIncreasesRange (java static)
pub fn holding_extendo_grip_increases_range(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (p0.call("getEntity", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_2, "net/minecraft/world/entity/player/Player"), &JvmVal::Int(0)).as_i32() == 0 { pc = 20; continue 'jvm; }
                pc = 13;
            }
            12 => {
                local_1 = (local_2).clone();
                pc = 21;
            }
            20 => {
                return;
            }
            21 => {
                local_2 = (local_1.call("getPersistentData", vec![])).clone();
                local_3 = (ctx.call_static("com/simibubi/create/AllItems", "EXTENDO_GRIP", vec![]).call("isIn", vec![local_1.call("getOffhandItem", vec![])])).clone();
                local_4 = (ctx.call_static("com/simibubi/create/AllItems", "EXTENDO_GRIP", vec![]).call("isIn", vec![local_1.call("getMainHandItem", vec![])])).clone();
                if jvm_cmp(&local_3, &JvmVal::Int(0)).as_i32() == 0 { pc = 62; continue 'jvm; }
                pc = 55;
            }
            53 => {
                if jvm_cmp(&local_4, &JvmVal::Int(0)).as_i32() == 0 { pc = 62; continue 'jvm; }
                pc = 59;
            }
            58 => {
                pc = 63;
            }
            62 => {
                pc = 65;
            }
            63 => {
                local_5 = (JvmVal::Int(0)).clone();
                local_6 = (jvm_bin("^", &local_3, &local_4)).clone();
                if jvm_cmp(&local_5, &JvmVal::Int(0)).as_i32() != 0 { pc = 82; continue 'jvm; }
                pc = 79;
            }
            78 => {
                pc = 83;
            }
            82 => {
                pc = 84;
            }
            83 => {
                local_6 = (jvm_bin("&", &JvmVal::Int(1), &JvmVal::Int(0))).clone();
                local_7 = (local_2.call("contains", vec![JvmVal::string("createExtendo")])).clone();
                local_8 = (local_2.call("contains", vec![JvmVal::string("createDualExtendo")])).clone();
                if jvm_cmp(&local_6, &local_7).as_i32() == 0 { pc = 173; continue 'jvm; }
                pc = 111;
            }
            109 => {
                if jvm_cmp(&local_6, &JvmVal::Int(0)).as_i32() != 0 { pc = 141; continue 'jvm; }
                pc = 115;
            }
            114 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "rangeModifier", vec![]).call("get", vec![]);
                local_1.call("getAttributes", vec![]).call("removeAttributeModifiers", vec![arg_tmp_0.clone()]);
                local_2.call("remove", vec![JvmVal::string("createExtendo")]);
                pc = 173;
            }
            141 => {
                ctx.call_static("com/simibubi/create/foundation/advancement/AllAdvancements", "EXTENDO_GRIP", vec![]).call("awardTo", vec![local_1.clone()]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "rangeModifier", vec![]).call("get", vec![]);
                local_1.call("getAttributes", vec![]).call("addTransientAttributeModifiers", vec![arg_tmp_0.clone()]);
                local_2.call("putBoolean", vec![JvmVal::string("createExtendo"), JvmVal::Int(1)]);
                pc = 175;
            }
            173 => {
                if jvm_cmp(&local_5, &local_8).as_i32() == 0 { pc = 244; continue 'jvm; }
                pc = 182;
            }
            180 => {
                if jvm_cmp(&local_5, &JvmVal::Int(0)).as_i32() != 0 { pc = 212; continue 'jvm; }
                pc = 186;
            }
            185 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "doubleRangeModifier", vec![]).call("get", vec![]);
                local_1.call("getAttributes", vec![]).call("removeAttributeModifiers", vec![arg_tmp_0.clone()]);
                local_2.call("remove", vec![JvmVal::string("createDualExtendo")]);
                pc = 244;
            }
            212 => {
                ctx.call_static("com/simibubi/create/foundation/advancement/AllAdvancements", "EXTENDO_GRIP_DUAL", vec![]).call("awardTo", vec![local_1.clone()]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "doubleRangeModifier", vec![]).call("get", vec![]);
                local_1.call("getAttributes", vec![]).call("addTransientAttributeModifiers", vec![arg_tmp_0.clone()]);
                local_2.call("putBoolean", vec![JvmVal::string("createDualExtendo"), JvmVal::Int(1)]);
                pc = 244;
            }
            244 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: addReachToJoiningPlayersHoldingExtendo (java static)
pub fn add_reach_to_joining_players_holding_extendo(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getEntity", vec![])).clone();
                local_2 = (local_1.call("getPersistentData", vec![])).clone();
                if jvm_cmp(&local_2.call("contains", vec![JvmVal::string("createDualExtendo")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 40; continue 'jvm; }
                pc = 20;
            }
            19 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "doubleRangeModifier", vec![]).call("get", vec![]);
                local_1.call("getAttributes", vec![]).call("addTransientAttributeModifiers", vec![arg_tmp_0.clone()]);
                pc = 67;
            }
            40 => {
                if jvm_cmp(&local_2.call("contains", vec![JvmVal::string("createExtendo")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 67; continue 'jvm; }
                pc = 50;
            }
            49 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "rangeModifier", vec![]).call("get", vec![]);
                local_1.call("getAttributes", vec![]).call("addTransientAttributeModifiers", vec![arg_tmp_0.clone()]);
                pc = 67;
            }
            67 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: dontMissEntitiesWhenYouHaveHighReachDistance (java static)
pub fn dont_miss_entities_when_you_have_high_reach_distance(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![])).clone();
                local_2 = (local_1.get_field("player")).clone();
                if jvm_is_null(&local_1.get_field("level")) { pc = 20; continue 'jvm; }
                pc = 17;
            }
            16 => {
                if !jvm_is_null(&local_2) { pc = 21; continue 'jvm; }
                pc = 21;
            }
            20 => {
                return;
            }
            21 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "isHoldingExtendoGrip", vec![local_2.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 29; continue 'jvm; }
                pc = 29;
            }
            28 => {
                return;
            }
            29 => {
                if jvm_cmp(&jvm_instanceof(&local_1.get_field("hitResult"), "net/minecraft/world/phys/BlockHitResult"), &JvmVal::Int(0)).as_i32() == 0 { pc = 53; continue 'jvm; }
                pc = 40;
            }
            39 => {
                if jvm_ref_eq(&local_1.get_field("hitResult").call("getType", vec![]), &ctx.call_static("net/minecraft/world/phys/HitResult$Type", "MISS", vec![])) { pc = 53; continue 'jvm; }
                pc = 53;
            }
            52 => {
                return;
            }
            53 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/entity/ai/attributes/Attributes", "BLOCK_INTERACTION_RANGE", vec![]);
                local_3 = (local_2.call("getAttributeValue", vec![arg_tmp_0.clone()])).clone();
                if jvm_cmp(&local_2.call("isCreative", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 74; continue 'jvm; }
                pc = 69;
            }
            68 => {
                local_3 = (jvm_bin("-", &local_3, &JvmVal::Double(0.5_f64))).clone();
                pc = 75;
            }
            74 => {
    let arg_tmp_0 = ctx.call_static("net/createmod/catnip/animation/AnimationTickHolder", "getPartialTicks", vec![]);
                local_5 = (local_2.call("getEyePosition", vec![arg_tmp_0.clone()])).clone();
                local_6 = (local_2.call("getViewVector", vec![JvmVal::Float(1_f32)])).clone();
                local_7 = (local_5.call("add", vec![jvm_bin("*", &local_6.get_field("x"), &local_3), jvm_bin("*", &local_6.get_field("y"), &local_3), jvm_bin("*", &local_6.get_field("z"), &local_3)])).clone();
                local_8 = (local_2.call("getBoundingBox", vec![]).call("expandTowards", vec![local_6.call("scale", vec![local_3.clone()])]).call("inflate", vec![JvmVal::Double(1_f64), JvmVal::Double(1_f64), JvmVal::Double(1_f64)])).clone();
                local_9 = (ctx.call_static("net/minecraft/world/entity/projectile/ProjectileUtil", "getEntityHitResult", vec![local_2.clone(), local_5.clone(), local_7.clone(), local_8.clone(), JvmVal::Null, jvm_bin("*", &local_3, &local_3)])).clone();
                if jvm_is_null(&local_9) { pc = 244; continue 'jvm; }
                pc = 166;
            }
            164 => {
                local_10 = (local_9.call("getEntity", vec![])).clone();
                local_11 = (local_9.call("getLocation", vec![])).clone();
                local_12 = (local_5.call("distanceToSqr", vec![local_11.clone()])).clone();
                if jvm_cmp(&jvm_cmp(&local_12, &jvm_bin("*", &local_3, &local_3)), &JvmVal::Int(0)).as_i32() < 0 { pc = 216; continue 'jvm; }
                pc = 197;
            }
            196 => {
                if jvm_is_null(&local_1.get_field("hitResult")) { pc = 216; continue 'jvm; }
                pc = 204;
            }
            203 => {
                if !jvm_ref_eq(&local_1.get_field("hitResult").call("getType", vec![]), &ctx.call_static("net/minecraft/world/phys/HitResult$Type", "MISS", vec![])) { pc = 244; continue 'jvm; }
                pc = 217;
            }
            216 => {
                // putfield: local_9 handled by the bridge object model
                if jvm_cmp(&jvm_instanceof(&local_10, "net/minecraft/world/entity/LivingEntity"), &JvmVal::Int(0)).as_i32() != 0 { pc = 238; continue 'jvm; }
                pc = 232;
            }
            230 => {
                if jvm_cmp(&jvm_instanceof(&local_10, "net/minecraft/world/entity/decoration/ItemFrame"), &JvmVal::Int(0)).as_i32() == 0 { pc = 244; continue 'jvm; }
                pc = 239;
            }
            238 => {
                // putfield: local_10 handled by the bridge object model
                pc = 244;
            }
            244 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: consumeDurabilityOnBlockBreak (java static)
pub fn consume_durability_on_block_break(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "findAndDamageExtendoGrip", vec![p0.call("getPlayer", vec![])]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: consumeDurabilityOnPlace (java static)
pub fn consume_durability_on_place(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getEntity", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_1, "net/minecraft/world/entity/player/Player"), &JvmVal::Int(0)).as_i32() == 0 { pc = 19; continue 'jvm; }
                pc = 13;
            }
            12 => {
                ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "findAndDamageExtendoGrip", vec![local_1.clone()]);
                pc = 19;
            }
            19 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: findAndDamageExtendoGrip (java static)
pub fn find_and_damage_extendo_grip(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_is_null(&p0) { pc = 5; continue 'jvm; }
                pc = 5;
            }
            4 => {
                return;
            }
            5 => {
                if jvm_cmp(&p0.call("level", vec![]).get_field("isClientSide"), &JvmVal::Int(0)).as_i32() == 0 { pc = 16; continue 'jvm; }
                pc = 16;
            }
            15 => {
                return;
            }
            16 => {
                local_1 = (ctx.call_static("net/minecraft/world/entity/EquipmentSlot", "MAINHAND", vec![])).clone();
                local_2 = (p0.call("getMainHandItem", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "EXTENDO_GRIP", vec![]).call("isIn", vec![local_2.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 44; continue 'jvm; }
                pc = 36;
            }
            35 => {
                local_2 = (p0.call("getOffhandItem", vec![])).clone();
                local_1 = (ctx.call_static("net/minecraft/world/entity/EquipmentSlot", "OFFHAND", vec![])).clone();
                pc = 47;
            }
            44 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "EXTENDO_GRIP", vec![]).call("isIn", vec![local_2.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 55; continue 'jvm; }
                pc = 55;
            }
            54 => {
                return;
            }
            55 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "maxUses", vec![]);
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/armor/BacktankUtil", "canAbsorbDamage", vec![p0.clone(), arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 72; continue 'jvm; }
                pc = 66;
            }
            65 => {
                local_2.call("hurtAndBreak", vec![JvmVal::Int(1), p0.clone(), local_1.clone()]);
                pc = 72;
            }
            72 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: isBarVisible (java instance)
pub fn is_bar_visible(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "maxUses", vec![]);
                return ctx.call_static("com/simibubi/create/content/equipment/armor/BacktankUtil", "isBarVisible", vec![p0.clone(), arg_tmp_0.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: getBarWidth (java instance)
pub fn get_bar_width(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "maxUses", vec![]);
                return ctx.call_static("com/simibubi/create/content/equipment/armor/BacktankUtil", "getBarWidth", vec![p0.clone(), arg_tmp_0.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: getBarColor (java instance)
pub fn get_bar_color(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "maxUses", vec![]);
                return ctx.call_static("com/simibubi/create/content/equipment/armor/BacktankUtil", "getBarColor", vec![p0.clone(), arg_tmp_0.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: maxUses (java static)
pub fn max_uses(ctx: &mut BridgeCtx) -> JvmVal {
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return ctx.call_static("com/simibubi/create/infrastructure/config/AllConfigs", "server", vec![]).get_field("equipment").get_field("maxExtendoGripActions").call("get", vec![]).call("intValue", vec![]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: doesSneakBypassUse (java instance)
pub fn does_sneak_bypass_use(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: bufferLivingAttackEvent (java static)
pub fn buffer_living_attack_event(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                // putstatic: mod-own statics are modeled at module scope

                local_1 = (p0.call("getSource", vec![])).clone();
                if !jvm_is_null(&local_1) { pc = 17; continue 'jvm; }
                pc = 17;
            }
            16 => {
                return;
            }
            17 => {
                local_2 = (local_1.call("getEntity", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_2, "net/minecraft/world/entity/player/Player"), &JvmVal::Int(0)).as_i32() == 0 { pc = 36; continue 'jvm; }
                pc = 30;
            }
            29 => {
                ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "findAndDamageExtendoGrip", vec![local_2.clone()]);
                pc = 36;
            }
            36 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: attacksByExtendoGripHaveMoreKnockback (java static)
pub fn attacks_by_extendo_grip_have_more_knockback(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "lastActiveDamageSource", vec![])) { pc = 7; continue 'jvm; }
                pc = 7;
            }
            6 => {
                return;
            }
            7 => {
                local_1 = (ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "lastActiveDamageSource", vec![]).call("getDirectEntity", vec![])).clone();
                // putstatic: mod-own statics are modeled at module scope

                if jvm_cmp(&jvm_instanceof(&local_1, "net/minecraft/world/entity/player/Player"), &JvmVal::Int(0)).as_i32() == 0 { pc = 33; continue 'jvm; }
                pc = 26;
            }
            25 => {
                local_2 = (local_1).clone();
                pc = 34;
            }
            33 => {
                return;
            }
            34 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "isHoldingExtendoGrip", vec![local_2.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 42; continue 'jvm; }
                pc = 42;
            }
            41 => {
                return;
            }
            42 => {
                p0.call("setStrength", vec![jvm_bin("+", &p0.call("getStrength", vec![]), &JvmVal::Float(2_f32))]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: isUncaughtClientInteraction (java static)
pub fn is_uncaught_client_interaction(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_cmp(&p0.call("distanceToSqr", vec![p1.clone()]), &JvmVal::Double(36_f64)), &JvmVal::Int(0)).as_i32() >= 0 { pc = 14; continue 'jvm; }
                pc = 13;
            }
            12 => {
                return JvmVal::Int(0);
            }
            14 => {
                if jvm_cmp(&p0.call("level", vec![]).get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 26; continue 'jvm; }
                pc = 25;
            }
            24 => {
                return JvmVal::Int(0);
            }
            26 => {
                if jvm_cmp(&jvm_instanceof(&p0, "net/minecraft/world/entity/player/Player"), &JvmVal::Int(0)).as_i32() != 0 { pc = 35; continue 'jvm; }
                pc = 34;
            }
            33 => {
                return JvmVal::Int(0);
            }
            35 => {
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: notifyServerOfLongRangeAttacks (java static)
pub fn notify_server_of_long_range_attacks(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getEntity", vec![])).clone();
                local_2 = (p0.call("getTarget", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "isUncaughtClientInteraction", vec![local_1.clone(), local_2.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 19; continue 'jvm; }
                pc = 19;
            }
            18 => {
                return;
            }
            19 => {
                local_3 = (local_1).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "isHoldingExtendoGrip", vec![local_3.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 47; continue 'jvm; }
                pc = 34;
            }
            31 => {
                ctx.call_static("net/createmod/catnip/platform/CatnipServices", "NETWORK", vec![]).call("sendToServer", vec![JvmVal::obj("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripInteractionPacket")]);
                pc = 47;
            }
            47 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: notifyServerOfLongRangeInteractions (java static)
pub fn notify_server_of_long_range_interactions(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getEntity", vec![])).clone();
                local_2 = (p0.call("getTarget", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "isUncaughtClientInteraction", vec![local_1.clone(), local_2.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 19; continue 'jvm; }
                pc = 19;
            }
            18 => {
                return;
            }
            19 => {
                local_3 = (local_1).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "isHoldingExtendoGrip", vec![local_3.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 51; continue 'jvm; }
                pc = 34;
            }
            31 => {
                ctx.call_static("net/createmod/catnip/platform/CatnipServices", "NETWORK", vec![]).call("sendToServer", vec![JvmVal::obj("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripInteractionPacket")]);
                pc = 51;
            }
            51 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: notifyServerOfLongRangeSpecificInteractions (java static)
pub fn notify_server_of_long_range_specific_interactions(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getEntity", vec![])).clone();
                local_2 = (p0.call("getTarget", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "isUncaughtClientInteraction", vec![local_1.clone(), local_2.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 19; continue 'jvm; }
                pc = 19;
            }
            18 => {
                return;
            }
            19 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "isHoldingExtendoGrip", vec![local_1.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 50; continue 'jvm; }
                pc = 29;
            }
            26 => {
                ctx.call_static("net/createmod/catnip/platform/CatnipServices", "NETWORK", vec![]).call("sendToServer", vec![JvmVal::obj("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripInteractionPacket")]);
                pc = 50;
            }
            50 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: isHoldingExtendoGrip (java static)
pub fn is_holding_extendo_grip(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (ctx.call_static("com/simibubi/create/AllItems", "EXTENDO_GRIP", vec![]).call("isIn", vec![p0.call("getOffhandItem", vec![])])).clone();
                local_2 = (ctx.call_static("com/simibubi/create/AllItems", "EXTENDO_GRIP", vec![]).call("isIn", vec![p0.call("getMainHandItem", vec![])])).clone();
                if jvm_cmp(&local_1, &JvmVal::Int(0)).as_i32() != 0 { pc = 30; continue 'jvm; }
                pc = 27;
            }
            26 => {
                if jvm_cmp(&local_2, &JvmVal::Int(0)).as_i32() == 0 { pc = 34; continue 'jvm; }
                pc = 31;
            }
            30 => {
                pc = 35;
            }
            34 => {
                pc = 36;
            }
            35 => {
                local_3 = (JvmVal::Int(0)).clone();
                return local_3;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: initializeClient (java instance)
pub fn initialize_client_2(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/item/render/SimpleCustomRenderer", "create", vec![p_this.clone(), JvmVal::obj("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItemRenderer")]);
                p_this.call("accept", vec![arg_tmp_0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: lambda$dontMissEntitiesWhenYouHaveHighReachDistance$2 (java static)
pub fn lambda_dont_miss_entities_when_you_have_high_reach_distance_2(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p0.call("isSpectator", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 18; continue 'jvm; }
                pc = 8;
            }
            7 => {
                if jvm_cmp(&p0.call("isPickable", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 18; continue 'jvm; }
                pc = 15;
            }
            14 => {
                pc = 19;
            }
            18 => {
                pc = 19;
            }
            19 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: lambda$static$1 (java static)
pub fn lambda_static_1(ctx: &mut BridgeCtx) -> JvmVal {
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "doubleRangeAttributeModifier", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/world/entity/ai/attributes/Attributes", "BLOCK_INTERACTION_RANGE", vec![]);
                return ctx.call_static("com/google/common/collect/ImmutableMultimap", "of", vec![arg_tmp_1.clone(), arg_tmp_0.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem.class :: lambda$static$0 (java static)
pub fn lambda_static_0(ctx: &mut BridgeCtx) -> JvmVal {
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem", "singleRangeAttributeModifier", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/world/entity/ai/attributes/Attributes", "BLOCK_INTERACTION_RANGE", vec![]);
                return ctx.call_static("com/google/common/collect/ImmutableMultimap", "of", vec![arg_tmp_1.clone(), arg_tmp_0.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/redstone/link/LinkHandler.class :: onBlockActivated (java static)
pub fn on_block_activated_2(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getLevel", vec![])).clone();
                local_2 = (p0.call("getPos", vec![])).clone();
                local_3 = (p0.call("getEntity", vec![])).clone();
                local_4 = (p0.call("getHand", vec![])).clone();
                if jvm_cmp(&local_3.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 35; continue 'jvm; }
                pc = 29;
            }
            28 => {
                if jvm_cmp(&local_3.call("isSpectator", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 36; continue 'jvm; }
                pc = 36;
            }
            35 => {
                return;
            }
            36 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/redstone/link/LinkBehaviour", "TYPE", vec![]);
                local_5 = (ctx.call_static("com/simibubi/create/foundation/blockEntity/behaviour/BlockEntityBehaviour", "get", vec![local_1.clone(), local_2.clone(), arg_tmp_0.clone()])).clone();
                if !jvm_is_null(&local_5) { pc = 55; continue 'jvm; }
                pc = 55;
            }
            54 => {
                return;
            }
            55 => {
                local_6 = (local_3.call("getItemInHand", vec![local_4.clone()])).clone();
                local_7 = (ctx.call_static("com/simibubi/create/foundation/utility/RaycastHelper", "rayTraceRange", vec![local_1.clone(), local_3.clone(), JvmVal::Double(10_f64)])).clone();
                if !jvm_is_null(&local_7) { pc = 79; continue 'jvm; }
                pc = 79;
            }
            78 => {
                return;
            }
            79 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "LINKED_CONTROLLER", vec![]).call("isIn", vec![local_6.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 91; continue 'jvm; }
                pc = 91;
            }
            90 => {
                return;
            }
            91 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WRENCH", vec![]).call("isIn", vec![local_6.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 103; continue 'jvm; }
                pc = 103;
            }
            102 => {
                return;
            }
            103 => {
                local_8 = (jvm_instanceof(&local_3, "net/neoforged/neoforge/common/util/FakePlayer")).clone();
                local_9 = (JvmVal::Int(0)).clone();
                if jvm_cmp(&local_8, &JvmVal::Int(0)).as_i32() == 0 { pc = 203; continue 'jvm; }
                pc = 118;
            }
            117 => {
                local_10 = (local_1.call("getBlockState", vec![local_2.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/phys/Vec3", "atLowerCornerOf", vec![local_2.clone()]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/phys/Vec3", "atLowerCornerOf", vec![local_7.call("getDirection", vec![]).call("getNormal", vec![])]).call("scale", vec![JvmVal::Double(0.25_f64)]);
                local_11 = (local_7.call("getLocation", vec![]).call("subtract", vec![arg_tmp_0.clone()]).call("add", vec![arg_tmp_0.clone()])).clone();
                if jvm_cmp(&jvm_cmp(&local_11.call("distanceToSqr", vec![local_5.get_field("firstSlot").call("getLocalOffset", vec![local_1.clone(), local_2.clone(), local_10.clone()])]), &local_11.call("distanceToSqr", vec![local_5.get_field("secondSlot").call("getLocalOffset", vec![local_1.clone(), local_2.clone(), local_10.clone()])])), &JvmVal::Int(0)).as_i32() <= 0 { pc = 200; continue 'jvm; }
                pc = 197;
            }
            196 => {
                pc = 201;
            }
            200 => {
                pc = 203;
            }
            201 => {
                local_9 = (JvmVal::Int(0)).clone();
                pc = 204;
            }
            203 => {
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), ctx.call_static("java/lang/Boolean", "valueOf", vec![JvmVal::Int(0)]));
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(1).as_i32(), ctx.call_static("java/lang/Boolean", "valueOf", vec![JvmVal::Int(1)]));
                local_10 = (ctx.call_static("java/util/Arrays", "asList", vec![jvm_array_new(0)]).call("iterator", vec![])).clone();
                pc = 233;
            }
            231 => {
                if jvm_cmp(&local_10.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 336; continue 'jvm; }
                pc = 243;
            }
            241 => {
                local_11 = (local_10.call("next", vec![]).call("booleanValue", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("java/lang/Boolean", "valueOf", vec![local_11.clone()]);
                if jvm_cmp(&local_5.call("testHit", vec![arg_tmp_0.clone(), local_7.call("getLocation", vec![])]), &JvmVal::Int(0)).as_i32() != 0 { pc = 286; continue 'jvm; }
                pc = 276;
            }
            274 => {
                if jvm_cmp(&local_8, &JvmVal::Int(0)).as_i32() == 0 { pc = 333; continue 'jvm; }
                pc = 281;
            }
            279 => {
                if jvm_cmp(&local_9, &local_11).as_i32() != 0 { pc = 333; continue 'jvm; }
                pc = 287;
            }
            286 => {
                if jvm_ref_eq(&p0.call("getSide", vec![]), &ctx.call_static("net/neoforged/fml/LogicalSide", "CLIENT", vec![])) { pc = 305; continue 'jvm; }
                pc = 298;
            }
            296 => {
                local_5.call("setFrequency", vec![local_11.clone(), local_6.clone()]);
                pc = 306;
            }
            305 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/sounds/SoundSource", "BLOCKS", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/sounds/SoundEvents", "ITEM_FRAME_ADD_ITEM", vec![]);
                local_1.call("playSound", vec![JvmVal::Null, local_2.clone(), arg_tmp_1.clone(), arg_tmp_0.clone(), JvmVal::Float(0.25_f32), JvmVal::Float(0.1_f32)]);
                pc = 336;
            }
            333 => {
                pc = 231;
            }
            336 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/redstone/displayLink/ClickToLinkBlockItem.class :: linkableItemAlwaysPlacesWhenUsed (java static)
pub fn linkable_item_always_places_when_used(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getItemStack", vec![])).clone();
                local_3 = (local_1.call("getItem", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_3, "com/simibubi/create/content/redstone/displayLink/ClickToLinkBlockItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 25; continue 'jvm; }
                pc = 18;
            }
            17 => {
                local_2 = (local_3).clone();
                pc = 26;
            }
            25 => {
                return;
            }
            26 => {
                if jvm_cmp(&p0.call("getLevel", vec![]).call("getBlockState", vec![p0.call("getPos", vec![])]).call("is", vec![local_2.call("getBlock", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 48; continue 'jvm; }
                pc = 48;
            }
            47 => {
                return;
            }
            48 => {
    let arg_tmp_0 = ctx.call_static("net/neoforged/neoforge/common/util/TriState", "FALSE", vec![]);
                p0.call("setUseBlock", vec![arg_tmp_0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/redstone/displayLink/ClickToLinkBlockItem.class :: useOn (java instance)
pub fn use_on(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut local_14: JvmVal = JvmVal::Null;
    let mut local_15: JvmVal = JvmVal::Null;
    let mut local_16: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (p0.call("getItemInHand", vec![])).clone();
                local_3 = (p0.call("getClickedPos", vec![])).clone();
                local_4 = (p0.call("getLevel", vec![])).clone();
                local_5 = (local_4.call("getBlockState", vec![local_3.clone()])).clone();
                local_6 = (p0.call("getPlayer", vec![])).clone();
                local_7 = (p_this.call("getMessageTranslationKey", vec![])).clone();
                local_8 = (p_this.call("getMaxDistanceFromSelection", vec![])).clone();
                if !jvm_is_null(&local_6) { pc = 51; continue 'jvm; }
                pc = 50;
            }
            47 => {
                return ctx.call_static("net/minecraft/world/InteractionResult", "FAIL", vec![]);
            }
            51 => {
                if jvm_cmp(&local_6.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 121; continue 'jvm; }
                pc = 60;
            }
            59 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLICK_TO_LINK_DATA", vec![]);
                if jvm_cmp(&local_2.call("has", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 121; continue 'jvm; }
                pc = 71;
            }
            69 => {
                if jvm_cmp(&local_4.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() == 0 { pc = 81; continue 'jvm; }
                pc = 80;
            }
            77 => {
                return ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
            }
            81 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string(format!("")), jvm_array_new(0)]);
                local_7.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLICK_TO_LINK_DATA", vec![]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/component/DataComponents", "BLOCK_ENTITY_DATA", vec![]);
                return ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
            }
            121 => {
                local_9 = (local_4.call("dimension", vec![]).call("location", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLICK_TO_LINK_DATA", vec![]);
                if jvm_cmp(&local_2.call("has", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 316; continue 'jvm; }
                pc = 142;
            }
            141 => {
                if jvm_cmp(&p_this.call("isValidTarget", vec![local_4.clone(), local_3.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 262; continue 'jvm; }
                pc = 152;
            }
            151 => {
                if jvm_cmp(&p_this.call("placeWhenInvalid", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 222; continue 'jvm; }
                pc = 159;
            }
            158 => {
                local_10 = (p_this.call("useOn", vec![p0.clone()])).clone();
                if jvm_cmp(&local_4.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 181; continue 'jvm; }
                pc = 175;
            }
            173 => {
                if !jvm_ref_eq(&local_10, &ctx.call_static("net/minecraft/world/InteractionResult", "FAIL", vec![])) { pc = 184; continue 'jvm; }
                pc = 183;
            }
            181 => {
                return local_10;
            }
            184 => {
                local_11 = (local_6.call("getItemInHand", vec![p0.call("getHand", vec![])])).clone();
                if jvm_cmp(&local_11.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 219; continue 'jvm; }
                pc = 204;
            }
            203 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLICK_TO_LINK_DATA", vec![]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/component/DataComponents", "BLOCK_ENTITY_DATA", vec![]);
                pc = 221;
            }
            219 => {
                return local_10;
            }
            222 => {
                if jvm_cmp(&local_4.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() == 0 { pc = 238; continue 'jvm; }
                pc = 233;
            }
            230 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "DENY", vec![]).call("playFrom", vec![local_6.clone()]);
                pc = 240;
            }
            238 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string(format!("")), jvm_array_new(0)]);
                local_7.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                return ctx.call_static("net/minecraft/world/InteractionResult", "FAIL", vec![]);
            }
            262 => {
                if jvm_cmp(&local_4.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() == 0 { pc = 274; continue 'jvm; }
                pc = 273;
            }
            270 => {
                return ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
            }
            274 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string(format!("")), jvm_array_new(0)]);
                local_7.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLICK_TO_LINK_DATA", vec![]);
                return ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
            }
            316 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLICK_TO_LINK_DATA", vec![]);
                local_10 = (local_2.call("get", vec![arg_tmp_0.clone()])).clone();
                local_11 = (local_10.call("selectedPos", vec![])).clone();
                local_12 = (local_10.call("selectedDim", vec![])).clone();
                if jvm_cmp(&local_5.call("canBeReplaced", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 359; continue 'jvm; }
                pc = 356;
            }
            355 => {
                pc = 360;
            }
            359 => {
                pc = 363;
            }
            360 => {
                local_13 = (p0.call("getClickedFace", vec![]).call("relative", vec![JvmVal::Int(0), JvmVal::Int(1)])).clone();
                if jvm_cmp(&local_8, &JvmVal::Int(-1)).as_i32() == 0 { pc = 424; continue 'jvm; }
                pc = 373;
            }
            371 => {
                if jvm_cmp(&local_11.call("closerThan", vec![local_13.clone(), jvm_conv(&local_8, "D")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 394; continue 'jvm; }
                pc = 386;
            }
            384 => {
                if jvm_cmp(&local_12.call("equals", vec![local_9.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 424; continue 'jvm; }
                pc = 396;
            }
            394 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string(format!("")), jvm_array_new(0)]).call("withStyle", vec![arg_tmp_0.clone()]);
                local_7.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                return ctx.call_static("net/minecraft/world/InteractionResult", "FAIL", vec![]);
            }
            424 => {
                local_14 = (JvmVal::obj("net/minecraft/nbt/CompoundTag")).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/nbt/NbtUtils", "writeBlockPos", vec![local_11.call("subtract", vec![local_13.clone()])]);
                ctx.call_static("net/createmod/catnip/nbt/NBTHelper", "writeResourceLocation", vec![local_14.clone(), JvmVal::string("TargetDimension"), local_12.clone()]);
                ctx.call_static("net/minecraft/world/level/block/entity/BlockEntity", "addEntityType", vec![local_14.clone(), p_this.call("getBlock", vec![]).call("getBlockEntityType", vec![])]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/item/component/CustomData", "of", vec![local_14.clone()]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/core/component/DataComponents", "BLOCK_ENTITY_DATA", vec![]);
                local_15 = (p_this.call("useOn", vec![p0.clone()])).clone();
                if jvm_cmp(&local_4.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 513; continue 'jvm; }
                pc = 507;
            }
            505 => {
                if !jvm_ref_eq(&local_15, &ctx.call_static("net/minecraft/world/InteractionResult", "FAIL", vec![])) { pc = 516; continue 'jvm; }
                pc = 515;
            }
            513 => {
                return local_15;
            }
            516 => {
                local_16 = (local_6.call("getItemInHand", vec![p0.call("getHand", vec![])])).clone();
                if jvm_cmp(&local_16.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 551; continue 'jvm; }
                pc = 536;
            }
            535 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLICK_TO_LINK_DATA", vec![]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/component/DataComponents", "BLOCK_ENTITY_DATA", vec![]);
                pc = 553;
            }
            551 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "GREEN", vec![]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string(format!("")), jvm_array_new(0)]).call("withStyle", vec![arg_tmp_0.clone()]);
                local_7.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                return local_15;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/redstone/displayLink/ClickToLinkBlockItem.class :: clientTick (java static)
pub fn client_tick_3(ctx: &mut BridgeCtx) {
    let mut local_0: JvmVal = JvmVal::Null;
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_0 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("player")).clone();
                if !jvm_is_null(&local_0) { pc = 12; continue 'jvm; }
                pc = 12;
            }
            11 => {
                return;
            }
            12 => {
                local_1 = (local_0.call("getMainHandItem", vec![])).clone();
                local_3 = (local_1.call("getItem", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_3, "com/simibubi/create/content/redstone/displayLink/ClickToLinkBlockItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 37; continue 'jvm; }
                pc = 30;
            }
            29 => {
                local_2 = (local_3).clone();
                pc = 38;
            }
            37 => {
                return;
            }
            38 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLICK_TO_LINK_DATA", vec![]);
                if jvm_cmp(&local_1.call("has", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 49; continue 'jvm; }
                pc = 49;
            }
            48 => {
                return;
            }
            49 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "CLICK_TO_LINK_DATA", vec![]);
                local_3 = (local_1.call("get", vec![arg_tmp_0.clone()]).call("selectedPos", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/redstone/displayLink/ClickToLinkBlockItem", "lastShownPos", vec![]);
                if jvm_cmp(&local_3.call("equals", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 85; continue 'jvm; }
                pc = 74;
            }
            73 => {
                // putstatic: mod-own statics are modeled at module scope

                // putstatic: mod-own statics are modeled at module scope

                pc = 88;
            }
            85 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/redstone/displayLink/ClickToLinkBlockItem", "lastShownAABB", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/redstone/displayLink/ClickToLinkBlockItem.class :: placeWhenInvalid (java instance)
pub fn place_when_invalid(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/redstone/displayLink/ClickToLinkBlockItem.class :: isValidTarget (java instance)
pub fn is_valid_target(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/redstone/displayLink/ClickToLinkBlockItem.class :: getSelectionBounds (java instance)
pub fn get_selection_bounds(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("level")).clone();
                local_3 = (local_2.call("getBlockState", vec![p0.clone()])).clone();
                local_4 = (local_3.call("getShape", vec![local_2.clone(), p0.clone()])).clone();
                if jvm_cmp(&local_4.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 42; continue 'jvm; }
                pc = 32;
            }
            29 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/BlockPos", "ZERO", vec![]);
                pc = 51;
            }
            42 => {
                pc = 51;
            }
            51 => {
                return local_4.call("bounds", vec![]).call("move", vec![p0.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/logistics/depot/EjectorTargetHandler.class :: rightClickingBlocksSelectsThem (java static)
pub fn right_clicking_blocks_selects_them_2(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentItem", vec![])) { pc = 7; continue 'jvm; }
                pc = 7;
            }
            6 => {
                return;
            }
            7 => {
                local_1 = (p0.call("getPos", vec![])).clone();
                local_2 = (p0.call("getLevel", vec![])).clone();
                if jvm_cmp(&local_2.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 25; continue 'jvm; }
                pc = 25;
            }
            24 => {
                return;
            }
            25 => {
                local_3 = (p0.call("getEntity", vec![])).clone();
                if jvm_is_null(&local_3) { pc = 48; continue 'jvm; }
                pc = 35;
            }
            34 => {
                if jvm_cmp(&local_3.call("isSpectator", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 48; continue 'jvm; }
                pc = 42;
            }
            41 => {
                if jvm_cmp(&local_3.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 49; continue 'jvm; }
                pc = 49;
            }
            48 => {
                return;
            }
            49 => {
                local_4 = (JvmVal::string("weighted_ejector.target_set")).clone();
                local_5 = (ctx.call_static("net/minecraft/ChatFormatting", "GOLD", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![local_4.clone(), jvm_array_new(0)]).call("withStyle", vec![local_5.clone()]);
                local_3.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                // putstatic: mod-own statics are modeled at module scope

                // putstatic: mod-own statics are modeled at module scope

                p0.call("setCanceled", vec![JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/depot/EjectorTargetHandler.class :: leftClickingBlocksDeselectsThem (java static)
pub fn left_clicking_blocks_deselects_them_2(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentItem", vec![])) { pc = 7; continue 'jvm; }
                pc = 7;
            }
            6 => {
                return;
            }
            7 => {
                if jvm_cmp(&p0.call("getLevel", vec![]).get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 18; continue 'jvm; }
                pc = 18;
            }
            17 => {
                return;
            }
            18 => {
                if jvm_cmp(&p0.call("getEntity", vec![]).call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 29; continue 'jvm; }
                pc = 29;
            }
            28 => {
                return;
            }
            29 => {
                local_1 = (p0.call("getPos", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]);
                if jvm_cmp(&local_1.call("equals", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 57; continue 'jvm; }
                pc = 45;
            }
            44 => {
                // putstatic: mod-own statics are modeled at module scope

                // putstatic: mod-own statics are modeled at module scope

                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                pc = 57;
            }
            57 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/depot/EjectorTargetHandler.class :: flushSettings (java static)
pub fn flush_settings_2(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (JvmVal::Int(0)).clone();
                local_2 = (JvmVal::Int(0)).clone();
                local_3 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("player")).clone();
                local_4 = (JvmVal::string("weighted_ejector.target_not_valid")).clone();
                local_5 = (ctx.call_static("net/minecraft/ChatFormatting", "WHITE", vec![])).clone();
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![])) { pc = 30; continue 'jvm; }
                pc = 28;
            }
            26 => {
                local_4 = (JvmVal::string("weighted_ejector.no_target")).clone();
                pc = 31;
            }
            30 => {
                local_6 = (ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "getValidTargetDirection", vec![p0.clone()])).clone();
                if !jvm_is_null(&local_6) { pc = 69; continue 'jvm; }
                pc = 42;
            }
            41 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![local_4.clone(), jvm_array_new(0)]).call("withStyle", vec![local_5.clone()]);
                local_3.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                // putstatic: mod-own statics are modeled at module scope

                // putstatic: mod-own statics are modeled at module scope

                return;
            }
            69 => {
                local_4 = (JvmVal::string("weighted_ejector.targeting")).clone();
                local_5 = (ctx.call_static("net/minecraft/ChatFormatting", "GREEN", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]).call("getX", vec![]);
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(0).as_i32(), ctx.call_static("java/lang/Integer", "valueOf", vec![arg_tmp_0.clone()]));
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]).call("getY", vec![]);
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(1).as_i32(), ctx.call_static("java/lang/Integer", "valueOf", vec![arg_tmp_0.clone()]));
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]).call("getZ", vec![]);
                jvm_array_store(&jvm_array_new(0), JvmVal::Int(2).as_i32(), ctx.call_static("java/lang/Integer", "valueOf", vec![arg_tmp_0.clone()]));
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![local_4.clone(), jvm_array_new(0)]).call("withStyle", vec![local_5.clone()]);
                local_3.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]);
                local_7 = (p0.call("subtract", vec![arg_tmp_0.clone()])).clone();
                local_1 = (ctx.call_static("java/lang/Math", "abs", vec![jvm_bin("+", &local_7.call("getX", vec![]), &local_7.call("getZ", vec![]))])).clone();
                local_2 = (jvm_neg(&local_7.call("getY", vec![]))).clone();
                ctx.call_static("net/createmod/catnip/platform/CatnipServices", "NETWORK", vec![]).call("sendToServer", vec![JvmVal::obj("com/simibubi/create/content/logistics/depot/EjectorPlacementPacket")]);
                // putstatic: mod-own statics are modeled at module scope

                // putstatic: mod-own statics are modeled at module scope

                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/depot/EjectorTargetHandler.class :: getValidTargetDirection (java static)
pub fn get_valid_target_direction(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![])) { pc = 8; continue 'jvm; }
                pc = 7;
            }
            6 => {
                return JvmVal::Null;
            }
            8 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/Direction$Axis", "Y", vec![]);
    let arg_tmp_1 = ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]);
                if jvm_cmp(&ctx.call_static("net/createmod/catnip/math/VecHelper", "onSameAxis", vec![p0.clone(), arg_tmp_1.clone(), arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 23; continue 'jvm; }
                pc = 22;
            }
            21 => {
                return JvmVal::Null;
            }
            23 => {
                local_1 = (jvm_bin("-", &ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]).call("getX", vec![]), &p0.call("getX", vec![]))).clone();
                local_2 = (jvm_bin("-", &ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]).call("getZ", vec![]), &p0.call("getZ", vec![]))).clone();
                local_3 = (ctx.call_static("com/simibubi/create/infrastructure/config/AllConfigs", "server", vec![]).get_field("kinetics").get_field("maxEjectorDistance").call("get", vec![]).call("intValue", vec![])).clone();
                if jvm_cmp(&ctx.call_static("java/lang/Math", "abs", vec![local_1.clone()]), &local_3).as_i32() > 0 { pc = 82; continue 'jvm; }
                pc = 75;
            }
            74 => {
                if jvm_cmp(&ctx.call_static("java/lang/Math", "abs", vec![local_2.clone()]), &local_3).as_i32() <= 0 { pc = 84; continue 'jvm; }
                pc = 83;
            }
            82 => {
                return JvmVal::Null;
            }
            84 => {
                if jvm_cmp(&local_1, &JvmVal::Int(0)).as_i32() != 0 { pc = 108; continue 'jvm; }
                pc = 89;
            }
            88 => {
                if jvm_cmp(&local_2, &JvmVal::Int(0)).as_i32() >= 0 { pc = 98; continue 'jvm; }
                pc = 95;
            }
            92 => {
                pc = 101;
            }
            98 => {
                pc = 104;
            }
            101 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/Direction$Axis", "Z", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/core/Direction$AxisDirection", "POSITIVE", vec![]);
                return ctx.call_static("net/minecraft/core/Direction", "get", vec![arg_tmp_1.clone(), arg_tmp_0.clone()]);
            }
            108 => {
                if jvm_cmp(&local_2, &JvmVal::Int(0)).as_i32() != 0 { pc = 132; continue 'jvm; }
                pc = 113;
            }
            112 => {
                if jvm_cmp(&local_1, &JvmVal::Int(0)).as_i32() >= 0 { pc = 122; continue 'jvm; }
                pc = 119;
            }
            116 => {
                pc = 125;
            }
            122 => {
                pc = 128;
            }
            125 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/Direction$Axis", "X", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/core/Direction$AxisDirection", "POSITIVE", vec![]);
                return ctx.call_static("net/minecraft/core/Direction", "get", vec![arg_tmp_1.clone(), arg_tmp_0.clone()]);
            }
            132 => {
                return JvmVal::Null;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/logistics/depot/EjectorTargetHandler.class :: tick (java static)
pub fn tick_2(ctx: &mut BridgeCtx) {
    let mut local_0: JvmVal = JvmVal::Null;
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_0 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("player")).clone();
                if !jvm_is_null(&local_0) { pc = 12; continue 'jvm; }
                pc = 12;
            }
            11 => {
                return;
            }
            12 => {
                local_1 = (local_0.call("getMainHandItem", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "WEIGHTED_EJECTOR", vec![]).call("isIn", vec![local_1.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 34; continue 'jvm; }
                pc = 28;
            }
            27 => {
                // putstatic: mod-own statics are modeled at module scope

                pc = 55;
            }
            34 => {
                if jvm_ref_eq(&local_1, &ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentItem", vec![])) { pc = 49; continue 'jvm; }
                pc = 42;
            }
            41 => {
                // putstatic: mod-own statics are modeled at module scope

                // putstatic: mod-own statics are modeled at module scope

                pc = 52;
            }
            49 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]);
                ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "drawOutline", vec![arg_tmp_0.clone()]);
                pc = 56;
            }
            55 => {
                ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "checkForWrench", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "drawArc", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/depot/EjectorTargetHandler.class :: drawArc (java static)
pub fn draw_arc(ctx: &mut BridgeCtx) {
    let mut local_0: JvmVal = JvmVal::Null;
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut local_14: JvmVal = JvmVal::Null;
    let mut local_15: JvmVal = JvmVal::Null;
    let mut local_16: JvmVal = JvmVal::Null;
    let mut local_17: JvmVal = JvmVal::Null;
    let mut local_18: JvmVal = JvmVal::Null;
    let mut local_19: JvmVal = JvmVal::Null;
    let mut local_20: JvmVal = JvmVal::Null;
    let mut local_21: JvmVal = JvmVal::Null;
    let mut local_22: JvmVal = JvmVal::Null;
    let mut local_23: JvmVal = JvmVal::Null;
    let mut local_24: JvmVal = JvmVal::Null;
    let mut local_25: JvmVal = JvmVal::Null;
    let mut local_26: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_0 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![])).clone();
                local_1 = (ctx.call_static("com/simibubi/create/AllItems", "WRENCH", vec![]).call("isIn", vec![local_0.get_field("player").call("getMainHandItem", vec![])])).clone();
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![])) { pc = 25; continue 'jvm; }
                pc = 25;
            }
            24 => {
                return;
            }
            25 => {
                if !jvm_is_null(&ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentItem", vec![])) { pc = 36; continue 'jvm; }
                pc = 32;
            }
            31 => {
                if jvm_cmp(&local_1, &JvmVal::Int(0)).as_i32() != 0 { pc = 36; continue 'jvm; }
                pc = 36;
            }
            35 => {
                return;
            }
            36 => {
                local_2 = (local_0.get_field("hitResult")).clone();
                if jvm_cmp(&jvm_instanceof(&local_2, "net/minecraft/world/phys/BlockHitResult"), &JvmVal::Int(0)).as_i32() == 0 { pc = 56; continue 'jvm; }
                pc = 49;
            }
            48 => {
                local_3 = (local_2).clone();
                pc = 57;
            }
            56 => {
                return;
            }
            57 => {
                if !jvm_ref_eq(&local_3.call("getType", vec![]), &ctx.call_static("net/minecraft/world/phys/HitResult$Type", "MISS", vec![])) { pc = 68; continue 'jvm; }
                pc = 68;
            }
            67 => {
                return;
            }
            68 => {
                local_4 = (local_3.call("getBlockPos", vec![])).clone();
                if jvm_cmp(&local_1, &JvmVal::Int(0)).as_i32() != 0 { pc = 89; continue 'jvm; }
                pc = 80;
            }
            78 => {
                local_4 = (local_4.call("relative", vec![local_3.call("getDirection", vec![])])).clone();
                pc = 92;
            }
            89 => {
                local_5 = (jvm_bin("-", &ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]).call("getX", vec![]), &local_4.call("getX", vec![]))).clone();
                local_6 = (jvm_bin("-", &ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]).call("getY", vec![]), &local_4.call("getY", vec![]))).clone();
                local_7 = (jvm_bin("-", &ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]).call("getZ", vec![]), &local_4.call("getZ", vec![]))).clone();
                if jvm_cmp(&ctx.call_static("java/lang/Math", "abs", vec![local_7.clone()]), &ctx.call_static("java/lang/Math", "abs", vec![local_5.clone()])).as_i32() <= 0 { pc = 148; continue 'jvm; }
                pc = 145;
            }
            144 => {
                pc = 150;
            }
            148 => {
                pc = 152;
            }
            150 => {
                local_8 = (local_5).clone();
                if jvm_cmp(&ctx.call_static("java/lang/Math", "abs", vec![local_7.clone()]), &ctx.call_static("java/lang/Math", "abs", vec![local_5.clone()])).as_i32() >= 0 { pc = 169; continue 'jvm; }
                pc = 166;
            }
            165 => {
                pc = 171;
            }
            169 => {
                pc = 173;
            }
            171 => {
                local_9 = (local_7).clone();
                local_10 = (ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]).call("offset", vec![local_8.clone(), local_6.clone(), local_9.clone()])).clone();
                local_11 = (ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "getValidTargetDirection", vec![local_10.clone()])).clone();
                if !jvm_is_null(&local_11) { pc = 200; continue 'jvm; }
                pc = 200;
            }
            199 => {
                return;
            }
            200 => {
                if jvm_is_null(&ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "launcher", vec![])) { pc = 218; continue 'jvm; }
                pc = 209;
            }
            206 => {
                if jvm_cmp(&jvm_cmp(&ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "lastHoveredBlockPos", vec![]), &local_4.call("asLong", vec![])), &JvmVal::Int(0)).as_i32() == 0 { pc = 246; continue 'jvm; }
                pc = 220;
            }
            218 => {
                // putstatic: mod-own statics are modeled at module scope

    let arg_tmp_0 = ctx.call_static("java/lang/Math", "abs", vec![jvm_bin("+", &local_8, &local_9)]);
                // putstatic: mod-own statics are modeled at module scope

                pc = 249;
            }
            246 => {
                local_12 = (jvm_bin("+", &ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "launcher", vec![]).call("getTotalFlyingTicks", vec![]), &JvmVal::Double(3_f64))).clone();
                local_14 = (jvm_bin("+", &jvm_bin("/", &jvm_conv(&local_12, "I"), &JvmVal::Int(3)), &JvmVal::Int(1))).clone();
                local_15 = (jvm_bin("/", &local_12, &jvm_conv(&local_14, "D"))).clone();
                if jvm_cmp(&local_5, &local_8).as_i32() != 0 { pc = 293; continue 'jvm; }
                pc = 284;
            }
            282 => {
                if jvm_cmp(&local_7, &local_9).as_i32() != 0 { pc = 293; continue 'jvm; }
                pc = 290;
            }
            289 => {
                pc = 294;
            }
            293 => {
                pc = 296;
            }
            294 => {
                local_17 = (JvmVal::Int(0)).clone();
                if jvm_cmp(&local_17, &JvmVal::Int(0)).as_i32() == 0 { pc = 307; continue 'jvm; }
                pc = 304;
            }
            301 => {
                pc = 310;
            }
            307 => {
                pc = 312;
            }
            310 => {
                local_18 = (JvmVal::Int(16740721)).clone();
                local_19 = (JvmVal::obj("net/createmod/catnip/theme/Color").call("asVectorF", vec![])).clone();
                local_20 = (JvmVal::obj("net/minecraft/core/particles/DustParticleOptions")).clone();
                local_21 = (local_0.get_field("level")).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]).call("offset", vec![jvm_neg(&local_8), jvm_neg(&local_6), jvm_neg(&local_9)]);
                local_22 = (JvmVal::obj("net/minecraft/world/phys/AABB").call("move", vec![arg_tmp_0.clone()])).clone();
                local_23 = (JvmVal::Int(0)).clone();
                pc = 405;
            }
            403 => {
                if jvm_cmp(&local_23, &local_14).as_i32() >= 0 { pc = 491; continue 'jvm; }
                pc = 413;
            }
            410 => {
                local_24 = (jvm_bin("+", &jvm_bin("%", &jvm_conv(&jvm_bin("/", &ctx.call_static("net/createmod/catnip/animation/AnimationTickHolder", "getRenderTime", vec![]), &JvmVal::Float(3_f32)), "D"), &local_15), &jvm_bin("*", &jvm_conv(&local_23, "D"), &local_15))).clone();
                local_26 = (ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "launcher", vec![]).call("getGlobalPos", vec![local_24.clone(), local_11.clone(), local_4.clone()]).call("add", vec![jvm_conv(&jvm_bin("-", &local_5, &local_8), "D"), JvmVal::Double(0_f64), jvm_conv(&jvm_bin("-", &local_7, &local_9), "D")])).clone();
                local_21.call("addParticle", vec![local_20.clone(), local_26.get_field("x"), local_26.get_field("y"), local_26.get_field("z"), JvmVal::Double(0_f64), JvmVal::Double(0_f64), JvmVal::Double(0_f64)]);
                local_23 = jvm_iinc(&local_23, 1);
                pc = 403;
            }
            491 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/depot/EjectorTargetHandler.class :: checkForWrench (java static)
pub fn check_for_wrench_2(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WRENCH", vec![]).call("isIn", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 11; continue 'jvm; }
                pc = 11;
            }
            10 => {
                return;
            }
            11 => {
                local_1 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("hitResult")).clone();
                if jvm_cmp(&jvm_instanceof(&local_1, "net/minecraft/world/phys/BlockHitResult"), &JvmVal::Int(0)).as_i32() == 0 { pc = 33; continue 'jvm; }
                pc = 26;
            }
            25 => {
                local_2 = (local_1).clone();
                pc = 34;
            }
            33 => {
                return;
            }
            34 => {
                local_3 = (local_2.call("getBlockPos", vec![])).clone();
                local_4 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("level").call("getBlockEntity", vec![local_3.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_4, "com/simibubi/create/content/logistics/depot/EjectorBlockEntity"), &JvmVal::Int(0)).as_i32() != 0 { pc = 70; continue 'jvm; }
                pc = 62;
            }
            59 => {
                // putstatic: mod-own statics are modeled at module scope

                // putstatic: mod-own statics are modeled at module scope

                return;
            }
            70 => {
                if jvm_cmp(&jvm_cmp(&ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "lastHoveredBlockPos", vec![]), &JvmVal::Long(-1i64)), &JvmVal::Int(0)).as_i32() == 0 { pc = 91; continue 'jvm; }
                pc = 83;
            }
            80 => {
                if jvm_cmp(&jvm_cmp(&ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "lastHoveredBlockPos", vec![]), &local_3.call("asLong", vec![])), &JvmVal::Int(0)).as_i32() == 0 { pc = 133; continue 'jvm; }
                pc = 93;
            }
            91 => {
                local_5 = (local_4).clone();
                if jvm_cmp(&local_5.call("getTargetPosition", vec![]).call("equals", vec![local_5.call("getBlockPos", vec![])]), &JvmVal::Int(0)).as_i32() != 0 { pc = 122; continue 'jvm; }
                pc = 116;
            }
            114 => {
                // putstatic: mod-own statics are modeled at module scope

                pc = 123;
            }
            122 => {
                // putstatic: mod-own statics are modeled at module scope

                // putstatic: mod-own statics are modeled at module scope

                pc = 136;
            }
            133 => {
                if jvm_cmp(&jvm_cmp(&ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "lastHoveredBlockPos", vec![]), &JvmVal::Long(-1i64)), &JvmVal::Int(0)).as_i32() == 0 { pc = 149; continue 'jvm; }
                pc = 146;
            }
            143 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "currentSelection", vec![]);
                ctx.call_static("com/simibubi/create/content/logistics/depot/EjectorTargetHandler", "drawOutline", vec![arg_tmp_0.clone()]);
                pc = 149;
            }
            149 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/depot/EjectorTargetHandler.class :: drawOutline (java static)
pub fn draw_outline(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("level")).clone();
                if !jvm_is_null(&p0) { pc = 12; continue 'jvm; }
                pc = 12;
            }
            11 => {
                return;
            }
            12 => {
                local_2 = (p0).clone();
                local_3 = (local_1.call("getBlockState", vec![local_2.clone()])).clone();
                local_4 = (local_3.call("getShape", vec![local_1.clone(), local_2.clone()])).clone();
                if jvm_cmp(&local_4.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 49; continue 'jvm; }
                pc = 39;
            }
            36 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/BlockPos", "ZERO", vec![]);
                pc = 54;
            }
            49 => {
                pc = 56;
            }
            54 => {
                local_5 = (local_4.call("bounds", vec![])).clone();
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/stockTicker/StockTickerInteractionHandler.class :: interactWithLogisticsManager (java static)
pub fn interact_with_logistics_manager(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getTarget", vec![])).clone();
                local_2 = (p0.call("getEntity", vec![])).clone();
                if jvm_is_null(&local_2) { pc = 18; continue 'jvm; }
                pc = 15;
            }
            14 => {
                if !jvm_is_null(&local_1) { pc = 19; continue 'jvm; }
                pc = 19;
            }
            18 => {
                return;
            }
            19 => {
                if jvm_cmp(&local_2.call("isSpectator", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 27; continue 'jvm; }
                pc = 27;
            }
            26 => {
                return;
            }
            27 => {
                local_3 = (p0.call("getLevel", vec![])).clone();
                local_4 = (ctx.call_static("com/simibubi/create/content/logistics/stockTicker/StockTickerInteractionHandler", "getStockTickerPosition", vec![local_1.clone()])).clone();
                if !jvm_is_null(&local_4) { pc = 44; continue 'jvm; }
                pc = 44;
            }
            43 => {
                return;
            }
            44 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/logistics/stockTicker/StockTickerInteractionHandler", "interactWithLogisticsManagerAt", vec![local_2.clone(), local_3.clone(), local_4.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 66; continue 'jvm; }
                pc = 55;
            }
            54 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                pc = 66;
            }
            66 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/stockTicker/StockTickerInteractionHandler.class :: interactWithLogisticsManagerAt (java static)
pub fn interact_with_logistics_manager_at(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_3 = (p0.call("getMainHandItem", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "SHOPPING_LIST", vec![]).call("isIn", vec![local_3.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 24; continue 'jvm; }
                pc = 16;
            }
            15 => {
                ctx.call_static("com/simibubi/create/content/logistics/stockTicker/StockTickerInteractionHandler", "interactWithShop", vec![p0.clone(), p1.clone(), p2.clone(), local_3.clone()]);
                return JvmVal::Int(1);
            }
            24 => {
                if jvm_cmp(&p1.call("isClientSide", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 33; continue 'jvm; }
                pc = 32;
            }
            31 => {
                return JvmVal::Int(1);
            }
            33 => {
                local_5 = (p1.call("getBlockEntity", vec![p2.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_5, "com/simibubi/create/content/logistics/stockTicker/StockTickerBlockEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 58; continue 'jvm; }
                pc = 50;
            }
            48 => {
                local_4 = (local_5).clone();
                pc = 60;
            }
            58 => {
                return JvmVal::Int(0);
            }
            60 => {
                if jvm_cmp(&local_4.get_field("behaviour").call("mayInteract", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 97; continue 'jvm; }
                pc = 73;
            }
            72 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("stock_keeper.locked"), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("component", vec![]);
                p0.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                return JvmVal::Int(1);
            }
            97 => {
                if jvm_cmp(&jvm_instanceof(&p0, "net/minecraft/server/level/ServerPlayer"), &JvmVal::Int(0)).as_i32() == 0 { pc = 203; continue 'jvm; }
                pc = 105;
            }
            104 => {
                local_5 = (p0).clone();
                if jvm_cmp(&local_4.get_field("behaviour").call("mayAdministrate", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 143; continue 'jvm; }
                pc = 125;
            }
            122 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/Create", "LOGISTICS", vec![]).call("isLockable", vec![local_4.get_field("behaviour").get_field("freqId")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 143; continue 'jvm; }
                pc = 140;
            }
            139 => {
                pc = 144;
            }
            143 => {
                pc = 146;
            }
            144 => {
                local_6 = (JvmVal::Int(0)).clone();
                local_7 = (ctx.call_static("com/simibubi/create/Create", "LOGISTICS", vec![]).call("isLocked", vec![local_4.get_field("behaviour").get_field("freqId")])).clone();
                local_4.call("getRecentSummary", vec![]).call("divideAndSendTo", vec![local_5.clone(), p2.clone()]);
                pc = 204;
            }
            203 => {
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/logistics/stockTicker/StockTickerInteractionHandler.class :: interactWithShop (java static)
pub fn interact_with_shop(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut local_14: JvmVal = JvmVal::Null;
    let mut local_15: JvmVal = JvmVal::Null;
    let mut local_16: JvmVal = JvmVal::Null;
    let mut local_17: JvmVal = JvmVal::Null;
    let mut local_18: JvmVal = JvmVal::Null;
    let mut local_19: JvmVal = JvmVal::Null;
    let mut local_20: JvmVal = JvmVal::Null;
    let mut local_21: JvmVal = JvmVal::Null;
    let mut local_22: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p1.call("isClientSide", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 8; continue 'jvm; }
                pc = 8;
            }
            7 => {
                return;
            }
            8 => {
                local_5 = (p1.call("getBlockEntity", vec![p2.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_5, "com/simibubi/create/content/logistics/stockTicker/StockTickerBlockEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 33; continue 'jvm; }
                pc = 25;
            }
            23 => {
                local_4 = (local_5).clone();
                pc = 34;
            }
            33 => {
                return;
            }
            34 => {
                local_5 = (ctx.call_static("com/simibubi/create/content/logistics/tableCloth/ShoppingListItem", "getList", vec![p3.clone()])).clone();
                if !jvm_is_null(&local_5) { pc = 46; continue 'jvm; }
                pc = 46;
            }
            45 => {
                return;
            }
            46 => {
                if jvm_cmp(&local_4.get_field("behaviour").get_field("freqId").call("equals", vec![local_5.call("shopNetwork", vec![])]), &JvmVal::Int(0)).as_i32() != 0 { pc = 96; continue 'jvm; }
                pc = 68;
            }
            65 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "DENY", vec![]).call("playOnServer", vec![p1.clone(), p0.call("blockPosition", vec![])]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("stock_keeper.wrong_network"), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("sendStatus", vec![p0.clone()]);
                return;
            }
            96 => {
                local_6 = (local_5.call("bakeEntries", vec![p1.clone(), JvmVal::Null])).clone();
                local_7 = (local_6.call("getSecond", vec![])).clone();
                local_8 = (local_6.call("getFirst", vec![])).clone();
                local_9 = (JvmVal::obj("com/simibubi/create/content/logistics/stockTicker/PackageOrder")).clone();
                local_10 = (local_4.call("getRecentSummary", vec![])).clone();
                local_11 = (local_9.call("stacks", vec![]).call("iterator", vec![])).clone();
                pc = 166;
            }
            164 => {
                if jvm_cmp(&local_11.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 239; continue 'jvm; }
                pc = 176;
            }
            174 => {
                local_12 = (local_11.call("next", vec![])).clone();
                if jvm_cmp(&local_10.call("getCountOf", vec![local_12.get_field("stack")]), &local_12.get_field("count")).as_i32() < 0 { pc = 207; continue 'jvm; }
                pc = 207;
            }
            204 => {
                pc = 164;
            }
            207 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "DENY", vec![]).call("playOnServer", vec![p1.clone(), p0.call("blockPosition", vec![])]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("stock_keeper.stock_level_too_low"), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("sendStatus", vec![p0.clone()]);
                return;
            }
            239 => {
                local_11 = (JvmVal::Int(0)).clone();
                local_12 = (local_7.call("getStacksByCount", vec![]).call("iterator", vec![])).clone();
                pc = 256;
            }
            254 => {
                if jvm_cmp(&local_12.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 303; continue 'jvm; }
                pc = 266;
            }
            264 => {
                local_13 = (local_12.call("next", vec![])).clone();
                local_11 = (jvm_bin("+", &local_11, &ctx.call_static("net/minecraft/util/Mth", "ceil", vec![jvm_bin("/", &jvm_conv(&local_13.get_field("count"), "F"), &jvm_conv(&local_13.get_field("stack").call("getMaxStackSize", vec![]), "F"))]))).clone();
                pc = 254;
            }
            303 => {
                local_12 = (JvmVal::Int(0)).clone();
                pc = 308;
            }
            306 => {
                if jvm_cmp(&local_12, &local_4.get_field("receivedPayments").call("getSlots", vec![])).as_i32() >= 0 { pc = 344; continue 'jvm; }
                pc = 321;
            }
            319 => {
                if jvm_cmp(&local_4.get_field("receivedPayments").call("getStackInSlot", vec![local_12.clone()]).call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 338; continue 'jvm; }
                pc = 338;
            }
            335 => {
                local_11 = jvm_iinc(&local_11, -1);
                pc = 341;
            }
            338 => {
                local_12 = jvm_iinc(&local_12, 1);
                pc = 306;
            }
            344 => {
                if jvm_cmp(&local_11, &JvmVal::Int(0)).as_i32() <= 0 { pc = 381; continue 'jvm; }
                pc = 352;
            }
            349 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "DENY", vec![]).call("playOnServer", vec![p1.clone(), p0.call("blockPosition", vec![])]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("stock_keeper.cash_register_full"), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("sendStatus", vec![p0.clone()]);
                return;
            }
            381 => {
                local_12 = (ctx.call_static("net/createmod/catnip/data/Iterate", "trueAndFalse", vec![])).clone();
                local_13 = (jvm_array_length(&local_12)).clone();
                local_14 = (JvmVal::Int(0)).clone();
                pc = 396;
            }
            394 => {
                if jvm_cmp(&local_14, &local_13).as_i32() >= 0 { pc = 642; continue 'jvm; }
                pc = 403;
            }
            401 => {
                local_15 = (jvm_array_load(&local_12, local_14.as_i32())).clone();
                local_16 = (local_7.call("copy", vec![])).clone();
                local_17 = (JvmVal::obj("java/util/ArrayList")).clone();
                local_18 = (JvmVal::Int(0)).clone();
                pc = 429;
            }
            427 => {
                if jvm_cmp(&local_18, &p0.call("getInventory", vec![]).get_field("items").call("size", vec![])).as_i32() >= 0 { pc = 569; continue 'jvm; }
                pc = 443;
            }
            442 => {
                local_19 = (p0.call("getInventory", vec![]).call("getItem", vec![local_18.clone()])).clone();
                if jvm_cmp(&local_19.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 464; continue 'jvm; }
                pc = 464;
            }
            461 => {
                pc = 563;
            }
            464 => {
                local_20 = (local_16.call("getCountOf", vec![local_19.clone()])).clone();
                if jvm_cmp(&local_20, &JvmVal::Int(0)).as_i32() != 0 { pc = 481; continue 'jvm; }
                pc = 481;
            }
            478 => {
                pc = 563;
            }
            481 => {
                local_21 = (ctx.call_static("java/lang/Math", "min", vec![local_19.call("getCount", vec![]), local_20.clone()])).clone();
                local_16.call("add", vec![local_19.clone(), jvm_neg(&local_21)]);
                if jvm_cmp(&local_15, &JvmVal::Int(0)).as_i32() == 0 { pc = 511; continue 'jvm; }
                pc = 511;
            }
            508 => {
                pc = 563;
            }
            511 => {
                local_22 = (jvm_bin("-", &local_19.call("getCount", vec![]), &local_21)).clone();
                if jvm_cmp(&local_22, &JvmVal::Int(0)).as_i32() != 0 { pc = 538; continue 'jvm; }
                pc = 535;
            }
            532 => {
                pc = 545;
            }
            538 => {
                pc = 548;
            }
            545 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/item/ItemStack", "EMPTY", vec![]);
                local_18.call("setItem", vec![arg_tmp_0.clone(), local_19.call("copyWithCount", vec![local_22.clone()])]);
                pc = 566;
            }
            563 => {
                local_18 = jvm_iinc(&local_18, 1);
                pc = 427;
            }
            569 => {
                if jvm_cmp(&local_15, &JvmVal::Int(0)).as_i32() == 0 { pc = 614; continue 'jvm; }
                pc = 576;
            }
            574 => {
                if jvm_cmp(&local_16.call("getTotalCount", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 614; continue 'jvm; }
                pc = 585;
            }
            582 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "DENY", vec![]).call("playOnServer", vec![p1.clone(), p0.call("blockPosition", vec![])]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
                ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translate", vec![JvmVal::string("stock_keeper.too_broke"), jvm_array_new(0)]).call("style", vec![arg_tmp_0.clone()]).call("sendStatus", vec![p0.clone()]);
                return;
            }
            614 => {
                if jvm_cmp(&local_15, &JvmVal::Int(0)).as_i32() == 0 { pc = 622; continue 'jvm; }
                pc = 622;
            }
            619 => {
                pc = 636;
            }
            622 => {
                local_4.call("forEach", vec![JvmVal::Null]);
                pc = 639;
            }
            636 => {
                local_14 = jvm_iinc(&local_14, 1);
                pc = 394;
            }
            642 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/tableCloth/ShoppingListItem", "getAddress", vec![p3.clone()]);
    let arg_tmp_1 = ctx.call_static("com/simibubi/create/content/logistics/packagerLink/LogisticallyLinkedBehaviour$RequestType", "PLAYER", vec![]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/item/ItemStack", "EMPTY", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/world/InteractionHand", "MAIN_HAND", vec![]);
                p0.call("setItemInHand", vec![arg_tmp_1.clone(), arg_tmp_0.clone()]);
                if jvm_cmp(&local_9.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 688; continue 'jvm; }
                pc = 679;
            }
            676 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "STOCK_TICKER_TRADE", vec![]).call("playOnServer", vec![p1.clone(), local_4.call("getBlockPos", vec![])]);
                pc = 688;
            }
            688 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/stockTicker/StockTickerInteractionHandler.class :: getStockTickerPosition (java static)
pub fn get_stock_ticker_position(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getRootVehicle", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_1, "com/simibubi/create/content/contraptions/actors/seat/SeatEntity"), &JvmVal::Int(0)).as_i32() != 0 { pc = 14; continue 'jvm; }
                pc = 13;
            }
            12 => {
                return JvmVal::Null;
            }
            14 => {
                if jvm_cmp(&jvm_instanceof(&p0, "net/minecraft/world/entity/LivingEntity"), &JvmVal::Int(0)).as_i32() != 0 { pc = 23; continue 'jvm; }
                pc = 22;
            }
            21 => {
                return JvmVal::Null;
            }
            23 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllEntityTypes", "PACKAGE", vec![]).call("is", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 35; continue 'jvm; }
                pc = 34;
            }
            33 => {
                return JvmVal::Null;
            }
            35 => {
                local_2 = (p0.call("blockPosition", vec![])).clone();
                local_3 = (JvmVal::Int(0)).clone();
                local_4 = (JvmVal::Null).clone();
                local_5 = (ctx.call_static("net/createmod/catnip/data/Iterate", "horizontalDirections", vec![])).clone();
                local_6 = (jvm_array_length(&local_5)).clone();
                local_7 = (JvmVal::Int(0)).clone();
                pc = 60;
            }
            58 => {
                if jvm_cmp(&local_7, &local_6).as_i32() >= 0 { pc = 152; continue 'jvm; }
                pc = 67;
            }
            65 => {
                local_8 = (jvm_array_load(&local_5, local_7.as_i32())).clone();
                local_9 = (ctx.call_static("net/createmod/catnip/data/Iterate", "zeroAndOne", vec![])).clone();
                local_10 = (jvm_array_length(&local_9)).clone();
                local_11 = (JvmVal::Int(0)).clone();
                pc = 87;
            }
            85 => {
                if jvm_cmp(&local_11, &local_10).as_i32() >= 0 { pc = 146; continue 'jvm; }
                pc = 94;
            }
            92 => {
                local_12 = (jvm_array_load(&local_9, local_11.as_i32())).clone();
                local_13 = (local_2.call("relative", vec![local_8.clone()]).call("above", vec![local_12.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&p0.call("level", vec![]).call("getBlockState", vec![local_13.clone()]).call("getBlock", vec![]), "com/simibubi/create/content/logistics/stockTicker/StockTickerBlock"), &JvmVal::Int(0)).as_i32() != 0 { pc = 133; continue 'jvm; }
                pc = 133;
            }
            130 => {
                pc = 140;
            }
            133 => {
                local_4 = (local_13).clone();
                local_3 = jvm_iinc(&local_3, 1);
                pc = 143;
            }
            140 => {
                local_11 = jvm_iinc(&local_11, 1);
                pc = 85;
            }
            146 => {
                local_7 = jvm_iinc(&local_7, 1);
                pc = 58;
            }
            152 => {
                if jvm_cmp(&local_3, &JvmVal::Int(1)).as_i32() == 0 { pc = 159; continue 'jvm; }
                pc = 158;
            }
            157 => {
                return JvmVal::Null;
            }
            159 => {
                return local_4;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/logistics/stockTicker/StockTickerInteractionHandler.class :: lambda$interactWithShop$1 (java static)
pub fn lambda_interact_with_shop_1(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/stockTicker/StockTickerInteractionHandler.class :: lambda$interactWithLogisticsManagerAt$0 (java static)
pub fn lambda_interact_with_logistics_manager_at_0(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/itemHatch/ItemHatchHandler.class :: useOnItemHatchIgnoresSneak (java static)
pub fn use_on_item_hatch_ignores_sneak(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if !jvm_ref_eq(&p0.call("getUseItem", vec![]), &ctx.call_static("net/neoforged/neoforge/common/util/TriState", "DEFAULT", vec![])) { pc = 37; continue 'jvm; }
                pc = 13;
            }
            10 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "ITEM_HATCH", vec![]).call("has", vec![p0.call("getLevel", vec![]).call("getBlockState", vec![p0.call("getPos", vec![])])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 37; continue 'jvm; }
                pc = 31;
            }
            30 => {
    let arg_tmp_0 = ctx.call_static("net/neoforged/neoforge/common/util/TriState", "TRUE", vec![]);
                p0.call("setUseBlock", vec![arg_tmp_0.clone()]);
                pc = 37;
            }
            37 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/funnel/FunnelItem.class :: funnelItemAlwaysPlacesWhenUsed (java static)
pub fn funnel_item_always_places_when_used(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&jvm_instanceof(&p0.call("getItemStack", vec![]).call("getItem", vec![]), "com/simibubi/create/content/logistics/funnel/FunnelItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 20; continue 'jvm; }
                pc = 14;
            }
            13 => {
    let arg_tmp_0 = ctx.call_static("net/neoforged/neoforge/common/util/TriState", "FALSE", vec![]);
                p0.call("setUseBlock", vec![arg_tmp_0.clone()]);
                pc = 20;
            }
            20 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/logistics/funnel/FunnelItem.class :: getPlacementState (java instance)
pub fn get_placement_state(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (p0.call("getLevel", vec![])).clone();
                local_3 = (p0.call("getClickedPos", vec![])).clone();
                local_4 = (p_this.call("getPlacementState", vec![p0.clone()])).clone();
                if !jvm_is_null(&local_4) { pc = 25; continue 'jvm; }
                pc = 24;
            }
            22 => {
                return local_4;
            }
            25 => {
                if jvm_cmp(&jvm_instanceof(&local_4.call("getBlock", vec![]), "com/simibubi/create/content/logistics/funnel/FunnelBlock"), &JvmVal::Int(0)).as_i32() != 0 { pc = 39; continue 'jvm; }
                pc = 38;
            }
            36 => {
                return local_4;
            }
            39 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/funnel/FunnelBlock", "FACING", vec![]);
                if jvm_cmp(&local_4.call("getValue", vec![arg_tmp_0.clone()]).call("getAxis", vec![]).call("isVertical", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 62; continue 'jvm; }
                pc = 61;
            }
            59 => {
                return local_4;
            }
            62 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/funnel/FunnelBlock", "FACING", vec![]);
                local_5 = (local_4.call("getValue", vec![arg_tmp_0.clone()])).clone();
                local_6 = (p_this.call("getBlock", vec![])).clone();
                local_7 = (local_6.call("getEquivalentBeltFunnel", vec![local_2.clone(), local_3.clone(), local_4.clone()]).call("getBlock", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/logistics/funnel/BeltFunnelBlock", "HORIZONTAL_FACING", vec![]);
                local_8 = (local_7.call("getStateForPlacement", vec![p0.clone()]).call("setValue", vec![arg_tmp_0.clone(), local_5.clone()])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/logistics/funnel/BeltFunnelBlock", "isOnValidBelt", vec![local_8.clone(), local_2.clone(), local_3.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 130; continue 'jvm; }
                pc = 129;
            }
            127 => {
                return local_8;
            }
            130 => {
                return local_4;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/trains/schedule/ScheduleItemEntityInteraction.class :: interactWithConductor (java static)
pub fn interact_with_conductor(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut local_14: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getTarget", vec![])).clone();
                local_2 = (p0.call("getEntity", vec![])).clone();
                if jvm_is_null(&local_2) { pc = 18; continue 'jvm; }
                pc = 15;
            }
            14 => {
                if !jvm_is_null(&local_1) { pc = 19; continue 'jvm; }
                pc = 19;
            }
            18 => {
                return;
            }
            19 => {
                if jvm_cmp(&local_2.call("isSpectator", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 27; continue 'jvm; }
                pc = 27;
            }
            26 => {
                return;
            }
            27 => {
                local_3 = (local_1.call("getRootVehicle", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_3, "com/simibubi/create/content/trains/entity/CarriageContraptionEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 48; continue 'jvm; }
                pc = 40;
            }
            39 => {
                local_4 = (local_3).clone();
                pc = 49;
            }
            48 => {
                return;
            }
            49 => {
                if jvm_cmp(&jvm_instanceof(&local_1, "net/minecraft/world/entity/LivingEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 65; continue 'jvm; }
                pc = 57;
            }
            56 => {
                local_5 = (local_1).clone();
                pc = 66;
            }
            65 => {
                return;
            }
            66 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllItems", "SCHEDULE", vec![]).call("get", vec![]);
                if jvm_cmp(&local_2.call("getCooldowns", vec![]).call("isOnCooldown", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 86; continue 'jvm; }
                pc = 86;
            }
            85 => {
                return;
            }
            86 => {
                local_6 = (p0.call("getItemStack", vec![])).clone();
                local_8 = (local_6.call("getItem", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_8, "com/simibubi/create/content/trains/schedule/ScheduleItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 167; continue 'jvm; }
                pc = 109;
            }
            107 => {
                local_7 = (local_8).clone();
                local_8 = (local_7.call("handScheduleTo", vec![local_6.clone(), local_2.clone(), local_5.clone(), p0.call("getHand", vec![])])).clone();
                if jvm_cmp(&local_8.call("consumesAction", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 167; continue 'jvm; }
                pc = 139;
            }
            138 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllItems", "SCHEDULE", vec![]).call("get", vec![]);
                local_2.call("getCooldowns", vec![]).call("addCooldown", vec![arg_tmp_0.clone(), JvmVal::Int(5)]);
                p0.call("setCancellationResult", vec![local_8.clone()]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                return;
            }
            167 => {
                if !jvm_ref_eq(&p0.call("getHand", vec![]), &ctx.call_static("net/minecraft/world/InteractionHand", "OFF_HAND", vec![])) { pc = 178; continue 'jvm; }
                pc = 178;
            }
            177 => {
                return;
            }
            178 => {
                local_7 = (local_4.call("getContraption", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_7, "com/simibubi/create/content/trains/entity/CarriageContraption"), &JvmVal::Int(0)).as_i32() == 0 { pc = 203; continue 'jvm; }
                pc = 195;
            }
            193 => {
                local_8 = (local_7).clone();
                pc = 204;
            }
            203 => {
                return;
            }
            204 => {
                local_9 = (local_4.call("getCarriage", vec![]).get_field("train")).clone();
                if !jvm_is_null(&local_9) { pc = 220; continue 'jvm; }
                pc = 220;
            }
            219 => {
                return;
            }
            220 => {
                if !jvm_is_null(&local_9.get_field("runtime").call("getSchedule", vec![])) { pc = 232; continue 'jvm; }
                pc = 232;
            }
            231 => {
                return;
            }
            232 => {
                local_10 = (local_7.call("getSeatMapping", vec![]).call("get", vec![local_1.call("getUUID", vec![])])).clone();
                if !jvm_is_null(&local_10) { pc = 257; continue 'jvm; }
                pc = 257;
            }
            256 => {
                return;
            }
            257 => {
                local_11 = (local_7.call("getSeats", vec![]).call("get", vec![local_10.call("intValue", vec![])])).clone();
                local_12 = (local_8.get_field("conductorSeats").call("get", vec![local_11.clone()])).clone();
                if !jvm_is_null(&local_12) { pc = 300; continue 'jvm; }
                pc = 300;
            }
            299 => {
                return;
            }
            300 => {
                if jvm_cmp(&p0.call("getLevel", vec![]).get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 314; continue 'jvm; }
                pc = 311;
            }
            310 => {
                pc = 315;
            }
            314 => {
                pc = 317;
            }
            315 => {
                local_13 = (JvmVal::Int(0)).clone();
                if jvm_cmp(&local_9.get_field("runtime").get_field("paused"), &JvmVal::Int(0)).as_i32() == 0 { pc = 413; continue 'jvm; }
                pc = 330;
            }
            328 => {
                if jvm_cmp(&local_9.get_field("runtime").get_field("completed"), &JvmVal::Int(0)).as_i32() != 0 { pc = 413; continue 'jvm; }
                pc = 341;
            }
            339 => {
                if jvm_cmp(&local_13, &JvmVal::Int(0)).as_i32() == 0 { pc = 383; continue 'jvm; }
                pc = 346;
            }
            344 => {
                // putfield: JvmVal::Int(0) handled by the bridge object model
                ctx.call_static("com/simibubi/create/AllSoundEvents", "CONFIRM", vec![]).call("playOnServer", vec![local_2.call("level", vec![]), local_2.call("blockPosition", vec![]), JvmVal::Float(1_f32), JvmVal::Float(1_f32)]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("schedule.continued"), jvm_array_new(0)]);
                local_2.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                pc = 384;
            }
            383 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllItems", "SCHEDULE", vec![]).call("get", vec![]);
                local_2.call("getCooldowns", vec![]).call("addCooldown", vec![arg_tmp_0.clone(), JvmVal::Int(5)]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                return;
            }
            413 => {
                local_14 = (local_2.call("getItemInHand", vec![p0.call("getHand", vec![])])).clone();
                if jvm_cmp(&local_14.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 479; continue 'jvm; }
                pc = 433;
            }
            431 => {
                if jvm_cmp(&local_13, &JvmVal::Int(0)).as_i32() == 0 { pc = 466; continue 'jvm; }
                pc = 439;
            }
            436 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "DENY", vec![]).call("playOnServer", vec![local_2.call("level", vec![]), local_2.call("blockPosition", vec![]), JvmVal::Float(1_f32), JvmVal::Float(1_f32)]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("schedule.remove_with_empty_hand"), jvm_array_new(0)]);
                local_2.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                pc = 467;
            }
            466 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                return;
            }
            479 => {
                if jvm_cmp(&local_13, &JvmVal::Int(0)).as_i32() == 0 { pc = 537; continue 'jvm; }
                pc = 485;
            }
            484 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "playItemPickup", vec![local_2.clone()]);
                if jvm_cmp(&local_9.get_field("runtime").get_field("isAutoSchedule"), &JvmVal::Int(0)).as_i32() == 0 { pc = 505; continue 'jvm; }
                pc = 502;
            }
            500 => {
                pc = 507;
            }
            505 => {
                pc = 508;
            }
            507 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("schedule.removed_from_train"), jvm_array_new(0)]);
                JvmVal::string("schedule.auto_removed_from_train").call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                local_2.call("getInventory", vec![]).call("placeItemBackInInventory", vec![local_9.get_field("runtime").call("returnSchedule", vec![local_2.call("registryAccess", vec![])])]);
                pc = 538;
            }
            537 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllItems", "SCHEDULE", vec![]).call("get", vec![]);
                local_2.call("getCooldowns", vec![]).call("addCooldown", vec![arg_tmp_0.clone(), JvmVal::Int(5)]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/trains/track/TrackBlockItem.class :: use (java instance)
pub fn use_(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (p1.call("getItemInHand", vec![p2.clone()])).clone();
                if jvm_cmp(&p1.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 31; continue 'jvm; }
                pc = 15;
            }
            14 => {
                if jvm_cmp(&p_this.call("isFoil", vec![local_4.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 31; continue 'jvm; }
                pc = 25;
            }
            23 => {
                return ctx.call_static("com/simibubi/create/content/trains/track/TrackBlockItem", "clearSelection", vec![local_4.clone(), p0.clone(), p1.clone()]);
            }
            31 => {
                return p_this.call("use", vec![p0.clone(), p1.clone(), p2.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/trains/track/TrackBlockItem.class :: useOn (java instance)
pub fn use_on_2(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (p0.call("getItemInHand", vec![])).clone();
                local_3 = (p0.call("getClickedPos", vec![])).clone();
                local_4 = (p0.call("getLevel", vec![])).clone();
                local_5 = (local_4.call("getBlockState", vec![local_3.clone()])).clone();
                local_6 = (p0.call("getPlayer", vec![])).clone();
                if !jvm_is_null(&local_6) { pc = 41; continue 'jvm; }
                pc = 36;
            }
            35 => {
                return p_this.call("useOn", vec![p0.clone()]);
            }
            41 => {
                if !jvm_ref_eq(&p0.call("getHand", vec![]), &ctx.call_static("net/minecraft/world/InteractionHand", "OFF_HAND", vec![])) { pc = 57; continue 'jvm; }
                pc = 52;
            }
            51 => {
                return p_this.call("useOn", vec![p0.clone()]);
            }
            57 => {
                local_7 = (local_6.call("getLookAngle", vec![])).clone();
                if jvm_cmp(&p_this.call("isFoil", vec![local_2.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 248; continue 'jvm; }
                pc = 74;
            }
            72 => {
                local_9 = (local_5.call("getBlock", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_9, "com/simibubi/create/content/trains/track/TrackBlock"), &JvmVal::Int(0)).as_i32() == 0 { pc = 146; continue 'jvm; }
                pc = 89;
            }
            87 => {
                local_8 = (local_9).clone();
                if jvm_cmp(&local_8.call("getTrackAxes", vec![local_4.clone(), local_3.clone(), local_5.clone()]).call("size", vec![]), &JvmVal::Int(1)).as_i32() <= 0 { pc = 146; continue 'jvm; }
                pc = 115;
            }
            113 => {
                if jvm_cmp(&local_4.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 142; continue 'jvm; }
                pc = 123;
            }
            121 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("track.junction_start"), jvm_array_new(0)]).call("withStyle", vec![arg_tmp_0.clone()]);
                local_6.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                pc = 145;
            }
            142 => {
                return ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
            }
            146 => {
                local_9 = (local_4.call("getBlockEntity", vec![local_3.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_9, "com/simibubi/create/content/trains/track/TrackBlockEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 210; continue 'jvm; }
                pc = 164;
            }
            162 => {
                local_8 = (local_9).clone();
                if jvm_cmp(&local_8.call("isTilted", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 210; continue 'jvm; }
                pc = 179;
            }
            177 => {
                if jvm_cmp(&local_4.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 206; continue 'jvm; }
                pc = 187;
            }
            185 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("track.turn_start"), jvm_array_new(0)]).call("withStyle", vec![arg_tmp_0.clone()]);
                local_6.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                pc = 209;
            }
            206 => {
                return ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
            }
            210 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/trains/track/TrackBlockItem", "select", vec![local_4.clone(), local_3.clone(), local_7.clone(), local_2.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 242; continue 'jvm; }
                pc = 224;
            }
            222 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/sounds/SoundSource", "BLOCKS", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/sounds/SoundEvents", "ITEM_FRAME_ADD_ITEM", vec![]);
                local_4.call("playSound", vec![JvmVal::Null, local_3.clone(), arg_tmp_1.clone(), arg_tmp_0.clone(), JvmVal::Float(0.75_f32), JvmVal::Float(1_f32)]);
                return ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
            }
            242 => {
                return p_this.call("useOn", vec![p0.clone()]);
            }
            248 => {
                if jvm_cmp(&local_6.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 268; continue 'jvm; }
                pc = 257;
            }
            256 => {
                return ctx.call_static("com/simibubi/create/content/trains/track/TrackBlockItem", "clearSelection", vec![local_2.clone(), local_4.clone(), local_6.clone()]).call("getResult", vec![]);
            }
            268 => {
                if jvm_cmp(&jvm_instanceof(&local_5.call("getBlock", vec![]), "com/simibubi/create/content/trains/track/ITrackBlock"), &JvmVal::Int(0)).as_i32() != 0 { pc = 283; continue 'jvm; }
                pc = 280;
            }
            279 => {
                pc = 284;
            }
            283 => {
                pc = 286;
            }
            284 => {
                local_8 = (JvmVal::Int(0)).clone();
    let arg_tmp_0 = ctx.call_static("java/lang/Boolean", "valueOf", vec![JvmVal::Int(0)]);
    let arg_tmp_1 = ctx.call_static("com/simibubi/create/AllDataComponents", "TRACK_EXTENDED_CURVE", vec![]);
                local_9 = (local_2.call("getOrDefault", vec![arg_tmp_1.clone(), arg_tmp_0.clone()]).call("booleanValue", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "TRACK_EXTENDED_CURVE", vec![]);
                if jvm_cmp(&local_8, &JvmVal::Int(0)).as_i32() == 0 { pc = 351; continue 'jvm; }
                pc = 320;
            }
            318 => {
                if jvm_cmp(&local_5.call("canBeReplaced", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 335; continue 'jvm; }
                pc = 327;
            }
            326 => {
                local_3 = (local_3.call("relative", vec![p0.call("getClickedFace", vec![])])).clone();
                pc = 336;
            }
            335 => {
                local_5 = (p_this.call("getPlacementState", vec![p0.clone()])).clone();
                if !jvm_is_null(&local_5) { pc = 351; continue 'jvm; }
                pc = 350;
            }
            347 => {
                return ctx.call_static("net/minecraft/world/InteractionResult", "FAIL", vec![]);
            }
            351 => {
                local_10 = (local_6.call("getOffhandItem", vec![])).clone();
                local_11 = (ctx.call_static("com/simibubi/create/AllBlocks", "METAL_GIRDER", vec![]).call("isIn", vec![local_10.clone()])).clone();
                local_12 = (ctx.call_static("com/simibubi/create/content/trains/track/TrackPlacement", "tryConnect", vec![local_4.clone(), local_6.clone(), local_3.clone(), local_5.clone(), local_2.clone(), local_11.clone(), local_9.clone()])).clone();
                if jvm_is_null(&local_12.get_field("message")) { pc = 419; continue 'jvm; }
                pc = 395;
            }
            393 => {
                if jvm_cmp(&local_4.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 419; continue 'jvm; }
                pc = 403;
            }
            401 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![local_12.get_field("message"), jvm_array_new(0)]);
                local_6.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                pc = 421;
            }
            419 => {
                if jvm_cmp(&local_12.get_field("valid"), &JvmVal::Int(0)).as_i32() != 0 { pc = 441; continue 'jvm; }
                pc = 430;
            }
            427 => {
                ctx.call_static("com/simibubi/create/AllSoundEvents", "DENY", vec![]).call("playFrom", vec![local_6.clone(), JvmVal::Float(1_f32), JvmVal::Float(1_f32)]);
                return ctx.call_static("net/minecraft/world/InteractionResult", "FAIL", vec![]);
            }
            441 => {
                if jvm_cmp(&local_4.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() == 0 { pc = 453; continue 'jvm; }
                pc = 452;
            }
            449 => {
                return ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
            }
            453 => {
                local_2 = (local_6.call("getMainHandItem", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllTags$AllBlockTags", "TRACKS", vec![]).call("matches", vec![local_2.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 495; continue 'jvm; }
                pc = 470;
            }
            469 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "TRACK_CONNECTING_FROM", vec![]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "TRACK_EXTENDED_CURVE", vec![]);
                local_6.call("setItemInHand", vec![p0.call("getHand", vec![]), local_2.clone()]);
                pc = 497;
            }
            495 => {
                local_13 = (local_5.call("getSoundType", vec![])).clone();
                if jvm_is_null(&local_13) { pc = 540; continue 'jvm; }
                pc = 509;
            }
            507 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/sounds/SoundSource", "BLOCKS", vec![]);
                local_4.call("playSound", vec![JvmVal::Null, local_3.clone(), local_13.call("getPlaceSound", vec![]), arg_tmp_0.clone(), jvm_bin("/", &jvm_bin("+", &local_13.call("getVolume", vec![]), &JvmVal::Float(1_f32)), &JvmVal::Float(2_f32)), jvm_bin("*", &local_13.call("getPitch", vec![]), &JvmVal::Float(0.8_f32))]);
                pc = 543;
            }
            540 => {
                return ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/trains/track/TrackBlockItem.class :: clearSelection (java static)
pub fn clear_selection(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p1.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() == 0 { pc = 28; continue 'jvm; }
                pc = 8;
            }
            7 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/sounds/SoundSource", "BLOCKS", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/sounds/SoundEvents", "ITEM_FRAME_REMOVE_ITEM", vec![]);
                p1.call("playSound", vec![p2.clone(), p2.call("blockPosition", vec![]), arg_tmp_1.clone(), arg_tmp_0.clone(), JvmVal::Float(0.75_f32), JvmVal::Float(1_f32)]);
                pc = 51;
            }
            28 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("track.selection_cleared"), jvm_array_new(0)]);
                p2.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "TRACK_CONNECTING_FROM", vec![]);
                pc = 52;
            }
            51 => {
                return ctx.call_static("net/minecraft/world/InteractionResultHolder", "sidedSuccess", vec![p0.clone(), p1.get_field("isClientSide")]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/trains/track/TrackBlockItem.class :: getPlacementState (java instance)
pub fn get_placement_state_2(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return p_this.call("getPlacementState", vec![p_this.call("updatePlacementContext", vec![JvmVal::obj("net/minecraft/world/item/context/BlockPlaceContext")])]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/trains/track/TrackBlockItem.class :: select (java static)
pub fn select(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (p0.call("getBlockState", vec![p1.clone()])).clone();
                local_5 = (local_4.call("getBlock", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_5, "com/simibubi/create/content/trains/track/ITrackBlock"), &JvmVal::Int(0)).as_i32() == 0 { pc = 34; continue 'jvm; }
                pc = 26;
            }
            24 => {
                local_6 = (local_5).clone();
                pc = 36;
            }
            34 => {
                return JvmVal::Int(0);
            }
            36 => {
                local_7 = (local_6.call("getNearestTrackAxis", vec![p0.clone(), p1.clone(), local_4.clone(), p2.clone()])).clone();
                if !jvm_ref_eq(&local_7.call("getSecond", vec![]), &ctx.call_static("net/minecraft/core/Direction$AxisDirection", "POSITIVE", vec![])) { pc = 75; continue 'jvm; }
                pc = 72;
            }
            69 => {
                pc = 76;
            }
            75 => {
                pc = 79;
            }
            76 => {
                local_8 = (JvmVal::Double(-1_f64).call("scale", vec![JvmVal::Double(1_f64)])).clone();
                local_9 = (local_6.call("getCurveStart", vec![p0.clone(), p1.clone(), local_4.clone(), local_8.clone()])).clone();
                local_10 = (local_6.call("getUpNormal", vec![p0.clone(), p1.clone(), local_4.clone()]).call("normalize", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "TRACK_CONNECTING_FROM", vec![]);
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/trains/track/TrackBlockItem.class :: sendExtenderPacket (java static)
pub fn send_extender_packet(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getItemStack", vec![])).clone();
                if jvm_cmp(&p0.call("getLevel", vec![]).get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 16; continue 'jvm; }
                pc = 16;
            }
            15 => {
                return;
            }
            16 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllTags$AllBlockTags", "TRACKS", vec![]).call("matches", vec![local_1.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 27; continue 'jvm; }
                pc = 27;
            }
            26 => {
                return;
            }
            27 => {
                if jvm_cmp(&ctx.call_static("net/minecraft/client/Minecraft", "getInstance", vec![]).get_field("options").get_field("keySprint").call("isDown", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 73; continue 'jvm; }
                pc = 45;
            }
            42 => {
                if !jvm_ref_eq(&p0.call("getHand", vec![]), &ctx.call_static("net/minecraft/world/InteractionHand", "MAIN_HAND", vec![])) { pc = 63; continue 'jvm; }
                pc = 60;
            }
            59 => {
                pc = 64;
            }
            63 => {
                pc = 65;
            }
            64 => {
                JvmVal::Int(1).call("sendToServer", vec![JvmVal::obj("com/simibubi/create/content/trains/track/PlaceExtendedCurvePacket")]);
                pc = 73;
            }
            73 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/trains/track/TrackBlockItem.class :: isFoil (java instance)
pub fn is_foil(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "TRACK_CONNECTING_FROM", vec![]);
                return p0.call("has", vec![arg_tmp_0.clone()]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/fluids/FluidBottleItemHook.class :: preventWaterBottlesFromCreatesFluids (java static)
pub fn prevent_water_bottles_from_creates_fluids(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getItemStack", vec![])).clone();
                if jvm_cmp(&local_1.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 13; continue 'jvm; }
                pc = 13;
            }
            12 => {
                return;
            }
            13 => {
                if jvm_cmp(&jvm_instanceof(&local_1.call("getItem", vec![]), "net/minecraft/world/item/BottleItem"), &JvmVal::Int(0)).as_i32() != 0 { pc = 24; continue 'jvm; }
                pc = 24;
            }
            23 => {
                return;
            }
            24 => {
                local_2 = (p0.call("getLevel", vec![])).clone();
                local_3 = (p0.call("getEntity", vec![])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/level/ClipContext$Fluid", "SOURCE_ONLY", vec![]);
                local_4 = (ctx.call_static("com/simibubi/create/content/fluids/FluidBottleItemHook", "getPlayerPOVHitResult", vec![local_2.clone(), local_3.clone(), arg_tmp_0.clone()])).clone();
                if jvm_ref_eq(&local_4.call("getType", vec![]), &ctx.call_static("net/minecraft/world/phys/HitResult$Type", "BLOCK", vec![])) { pc = 56; continue 'jvm; }
                pc = 56;
            }
            55 => {
                return;
            }
            56 => {
                local_5 = (local_4.call("getBlockPos", vec![])).clone();
                if jvm_cmp(&local_2.call("mayInteract", vec![local_3.clone(), local_5.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 77; continue 'jvm; }
                pc = 77;
            }
            76 => {
                return;
            }
            77 => {
                local_6 = (local_2.call("getFluidState", vec![local_5.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/tags/FluidTags", "WATER", vec![]);
                if jvm_cmp(&local_6.call("is", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 128; continue 'jvm; }
                pc = 98;
            }
            96 => {
                if jvm_cmp(&ctx.call_static("net/createmod/catnip/registry/RegisteredObjectsHelper", "getKeyOrThrow", vec![local_6.call("getType", vec![])]).call("getNamespace", vec![]).call("equals", vec![JvmVal::string("create")]), &JvmVal::Int(0)).as_i32() == 0 { pc = 128; continue 'jvm; }
                pc = 116;
            }
            115 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "PASS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                return;
            }
            128 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem.class :: rideable (java static)
pub fn rideable(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/entity/vehicle/AbstractMinecart$Type", "RIDEABLE", vec![]);
                return JvmVal::obj("com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem.class :: furnace (java static)
pub fn furnace(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/entity/vehicle/AbstractMinecart$Type", "FURNACE", vec![]);
                return JvmVal::obj("com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem.class :: chest (java static)
pub fn chest(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/entity/vehicle/AbstractMinecart$Type", "CHEST", vec![]);
                return JvmVal::obj("com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem.class :: canFitInsideContainerItems (java instance)
pub fn can_fit_inside_container_items(ctx: &mut BridgeCtx, p_this: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return ctx.call_static("com/simibubi/create/infrastructure/config/AllConfigs", "server", vec![]).get_field("kinetics").get_field("minecartContraptionInContainers").call("get", vec![]).call("booleanValue", vec![]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem.class :: useOn (java instance)
pub fn use_on_3(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (p0.call("getLevel", vec![])).clone();
                local_3 = (p0.call("getClickedPos", vec![])).clone();
                local_4 = (local_2.call("getBlockState", vec![local_3.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/tags/BlockTags", "RAILS", vec![]);
                if jvm_cmp(&local_4.call("is", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 32; continue 'jvm; }
                pc = 31;
            }
            28 => {
                return ctx.call_static("net/minecraft/world/InteractionResult", "FAIL", vec![]);
            }
            32 => {
                local_5 = (p0.call("getItemInHand", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_2, "net/minecraft/server/level/ServerLevel"), &JvmVal::Int(0)).as_i32() == 0 { pc = 202; continue 'jvm; }
                pc = 46;
            }
            45 => {
                local_6 = (local_2).clone();
                if jvm_cmp(&jvm_instanceof(&local_4.call("getBlock", vec![]), "net/minecraft/world/level/block/BaseRailBlock"), &JvmVal::Int(0)).as_i32() == 0 { pc = 81; continue 'jvm; }
                pc = 64;
            }
            62 => {
                pc = 84;
            }
            81 => {
                pc = 86;
            }
            84 => {
                local_7 = (ctx.call_static("net/minecraft/world/level/block/state/properties/RailShape", "NORTH_SOUTH", vec![])).clone();
                local_8 = (JvmVal::Double(0_f64)).clone();
                if jvm_cmp(&local_7.call("isAscending", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 102; continue 'jvm; }
                pc = 100;
            }
            97 => {
                local_8 = (JvmVal::Double(0.5_f64)).clone();
                pc = 104;
            }
            102 => {
                local_10 = (ctx.call_static("net/minecraft/world/entity/vehicle/AbstractMinecart", "createMinecart", vec![local_6.clone(), jvm_bin("+", &jvm_conv(&local_3.call("getX", vec![]), "D"), &JvmVal::Double(0.5_f64)), jvm_bin("+", &jvm_bin("+", &jvm_conv(&local_3.call("getY", vec![]), "D"), &JvmVal::Double(0.0625_f64)), &local_8), jvm_bin("+", &jvm_conv(&local_3.call("getZ", vec![]), "D"), &JvmVal::Double(0.5_f64)), p_this.get_field("minecartType"), local_5.clone(), JvmVal::Null])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/component/DataComponents", "CUSTOM_NAME", vec![]);
                if jvm_cmp(&local_5.call("has", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 167; continue 'jvm; }
                pc = 159;
            }
            157 => {
                local_10.call("setCustomName", vec![local_5.call("getHoverName", vec![])]);
                pc = 168;
            }
            167 => {
                local_11 = (p0.call("getPlayer", vec![])).clone();
                if !jvm_is_null(&local_11) { pc = 194; continue 'jvm; }
                pc = 191;
            }
            190 => {
                pc = 199;
            }
            194 => {
                pc = 202;
            }
            199 => {
                ctx.call_static("com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem", "addContraptionToMinecart", vec![local_5.clone(), local_10.clone(), JvmVal::Null, local_11.call("getDirection", vec![])]);
                pc = 204;
            }
            202 => {
                local_5.call("shrink", vec![JvmVal::Int(1)]);
                return ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem.class :: addContraptionToMinecart (java static)
pub fn add_contraption_to_minecart(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "MINECRAFT_CONTRAPTION_DATA", vec![]);
                if jvm_cmp(&p1.call("has", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 105; continue 'jvm; }
                pc = 11;
            }
            10 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "MINECRAFT_CONTRAPTION_DATA", vec![]);
                local_4 = (p1.call("get", vec![arg_tmp_0.clone()])).clone();
                local_5 = (ctx.call_static("net/createmod/catnip/nbt/NBTHelper", "readEnum", vec![local_4.clone(), JvmVal::string("InitialOrientation"), JvmVal::Null])).clone();
                local_6 = (ctx.call_static("com/simibubi/create/content/contraptions/Contraption", "fromNBT", vec![p0.clone(), local_4.clone(), JvmVal::Int(0)])).clone();
                if !jvm_is_null(&p3) { pc = 60; continue 'jvm; }
                pc = 50;
            }
            49 => {
                pc = 72;
            }
            60 => {
                pc = 74;
            }
            72 => {
                local_7 = (ctx.call_static("com/simibubi/create/content/contraptions/OrientedContraptionEntity", "createAtYaw", vec![p0.clone(), local_6.clone(), local_5.clone(), p3.call("toYRot", vec![])])).clone();
                local_7.call("setPos", vec![p2.call("getX", vec![]), p2.call("getY", vec![]), p2.call("getZ", vec![])]);
                pc = 105;
            }
            105 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem.class :: getDescriptionId (java instance)
pub fn get_description_id(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::string("item.create.minecart_contraption");
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem.class :: wrenchCanBeUsedToPickUpMinecartContraptions (java static)
pub fn wrench_can_be_used_to_pick_up_minecart_contraptions(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getTarget", vec![])).clone();
                local_2 = (p0.call("getEntity", vec![])).clone();
                if jvm_is_null(&local_2) { pc = 18; continue 'jvm; }
                pc = 15;
            }
            14 => {
                if !jvm_is_null(&local_1) { pc = 19; continue 'jvm; }
                pc = 19;
            }
            18 => {
                return;
            }
            19 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/infrastructure/config/AllConfigs", "server", vec![]).get_field("kinetics").get_field("survivalContraptionPickup").call("get", vec![]).call("booleanValue", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 48; continue 'jvm; }
                pc = 41;
            }
            40 => {
                if jvm_cmp(&local_2.call("isCreative", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 48; continue 'jvm; }
                pc = 48;
            }
            47 => {
                return;
            }
            48 => {
                local_3 = (local_2.call("getItemInHand", vec![p0.call("getHand", vec![])])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WRENCH", vec![]).call("isIn", vec![local_3.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 68; continue 'jvm; }
                pc = 68;
            }
            67 => {
                return;
            }
            68 => {
                if jvm_cmp(&jvm_instanceof(&local_1, "com/simibubi/create/content/contraptions/AbstractContraptionEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 80; continue 'jvm; }
                pc = 76;
            }
            75 => {
                local_1 = (local_1.call("getVehicle", vec![])).clone();
                pc = 81;
            }
            80 => {
                if jvm_cmp(&jvm_instanceof(&local_1, "net/minecraft/world/entity/vehicle/AbstractMinecart"), &JvmVal::Int(0)).as_i32() == 0 { pc = 96; continue 'jvm; }
                pc = 88;
            }
            87 => {
                local_4 = (local_1).clone();
                pc = 97;
            }
            96 => {
                return;
            }
            97 => {
                if jvm_cmp(&local_1.call("isAlive", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 105; continue 'jvm; }
                pc = 105;
            }
            104 => {
                return;
            }
            105 => {
                if jvm_cmp(&jvm_instanceof(&local_2, "com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer"), &JvmVal::Int(0)).as_i32() == 0 { pc = 127; continue 'jvm; }
                pc = 113;
            }
            112 => {
                local_5 = (local_2).clone();
                if jvm_cmp(&local_5.get_field("onMinecartContraption"), &JvmVal::Int(0)).as_i32() == 0 { pc = 127; continue 'jvm; }
                pc = 127;
            }
            126 => {
                return;
            }
            127 => {
                local_5 = (local_4.call("getMinecartType", vec![])).clone();
                if jvm_ref_eq(&local_5, &ctx.call_static("net/minecraft/world/entity/vehicle/AbstractMinecart$Type", "RIDEABLE", vec![])) { pc = 159; continue 'jvm; }
                pc = 144;
            }
            142 => {
                if jvm_ref_eq(&local_5, &ctx.call_static("net/minecraft/world/entity/vehicle/AbstractMinecart$Type", "FURNACE", vec![])) { pc = 159; continue 'jvm; }
                pc = 152;
            }
            150 => {
                if jvm_ref_eq(&local_5, &ctx.call_static("net/minecraft/world/entity/vehicle/AbstractMinecart$Type", "CHEST", vec![])) { pc = 159; continue 'jvm; }
                pc = 159;
            }
            158 => {
                return;
            }
            159 => {
                local_6 = (local_4.call("getPassengers", vec![])).clone();
                if jvm_cmp(&local_6.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 204; continue 'jvm; }
                pc = 178;
            }
            176 => {
                local_8 = (local_6.call("get", vec![JvmVal::Int(0)])).clone();
                if jvm_cmp(&jvm_instanceof(&local_8, "com/simibubi/create/content/contraptions/OrientedContraptionEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 204; continue 'jvm; }
                pc = 196;
            }
            194 => {
                local_7 = (local_8).clone();
                pc = 205;
            }
            204 => {
                return;
            }
            205 => {
                local_8 = (local_7.call("getContraption", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/api/contraption/ContraptionMovementSetting", "isNoPickup", vec![local_8.call("getBlocks", vec![]).call("values", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 250; continue 'jvm; }
                pc = 229;
            }
            228 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("contraption.minecart_contraption_illegal_pickup"), jvm_array_new(0)]).call("withStyle", vec![arg_tmp_0.clone()]);
                local_2.call("displayClientMessage", vec![arg_tmp_0.clone(), JvmVal::Int(1)]);
                return;
            }
            250 => {
                if jvm_cmp(&p0.call("getLevel", vec![]).get_field("isClientSide"), &JvmVal::Int(0)).as_i32() == 0 { pc = 273; continue 'jvm; }
                pc = 261;
            }
            260 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                return;
            }
            273 => {
                local_8.call("stop", vec![p0.call("getLevel", vec![])]);
                local_9 = (local_8.call("getActors", vec![]).call("iterator", vec![])).clone();
                pc = 296;
            }
            294 => {
                if jvm_cmp(&local_9.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 368; continue 'jvm; }
                pc = 306;
            }
            304 => {
                local_10 = (local_9.call("next", vec![])).clone();
                local_12 = (ctx.call_static("com/simibubi/create/api/behaviour/movement/MovementBehaviour", "REGISTRY", vec![]).call("get", vec![local_10.get_field("left").call("state", vec![])])).clone();
                if jvm_cmp(&jvm_instanceof(&local_12, "com/simibubi/create/content/contraptions/actors/psi/PortableStorageInterfaceMovement"), &JvmVal::Int(0)).as_i32() == 0 { pc = 365; continue 'jvm; }
                pc = 347;
            }
            345 => {
                local_11 = (local_12).clone();
                local_11.call("reset", vec![local_10.get_field("right")]);
                pc = 368;
            }
            365 => {
                pc = 294;
            }
            368 => {
                local_9 = (ctx.call_static("com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem", "create", vec![local_5.clone(), local_7.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/component/DataComponents", "CUSTOM_NAME", vec![]);
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/contraptions/data/ContraptionPickupLimiting", "isTooLargeForPickup", vec![local_9.call("saveOptional", vec![p0.call("getLevel", vec![]).call("registryAccess", vec![])])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 434; continue 'jvm; }
                pc = 411;
            }
            408 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/ChatFormatting", "RED", vec![]);
                local_10 = (ctx.call_static("com/simibubi/create/foundation/utility/CreateLang", "translateDirect", vec![JvmVal::string("contraption.minecart_contraption_too_big"), jvm_array_new(0)]).call("withStyle", vec![arg_tmp_0.clone()])).clone();
                local_2.call("displayClientMessage", vec![local_10.clone(), JvmVal::Int(1)]);
                return;
            }
            434 => {
                if jvm_cmp(&local_8.call("getBlocks", vec![]).call("size", vec![]), &JvmVal::Int(200)).as_i32() <= 0 { pc = 457; continue 'jvm; }
                pc = 453;
            }
            450 => {
                ctx.call_static("com/simibubi/create/foundation/advancement/AllAdvancements", "CART_PICKUP", vec![]).call("awardTo", vec![local_2.clone()]);
                pc = 458;
            }
            457 => {
                local_2.call("getInventory", vec![]).call("placeItemBackInInventory", vec![local_9.clone()]);
                local_7.call("discard", vec![]);
                local_1.call("discard", vec![]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem.class :: create (java static)
pub fn create(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (ctx.call_static("net/minecraft/world/item/ItemStack", "EMPTY", vec![])).clone();
                match jvm_array_load(&ctx.call_static("com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem$2", "$SwitchMap$net$minecraft$world$entity$vehicle$AbstractMinecart$Type", vec![]), p0.call("ordinal", vec![]).as_i32()).as_i32() {
                    1 => { pc = 40; }
                    2 => { pc = 50; }
                    3 => { pc = 60; }
                    _ => { pc = 70; }
                }
            }
            40 => {
                local_2 = (ctx.call_static("com/simibubi/create/AllItems", "MINECART_CONTRAPTION", vec![]).call("asStack", vec![])).clone();
                pc = 70;
            }
            50 => {
                local_2 = (ctx.call_static("com/simibubi/create/AllItems", "FURNACE_MINECART_CONTRAPTION", vec![]).call("asStack", vec![])).clone();
                pc = 70;
            }
            60 => {
                local_2 = (ctx.call_static("com/simibubi/create/AllItems", "CHEST_MINECART_CONTRAPTION", vec![]).call("asStack", vec![])).clone();
                pc = 70;
            }
            70 => {
                if jvm_cmp(&local_2.call("isEmpty", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 79; continue 'jvm; }
                pc = 78;
            }
            77 => {
                return local_2;
            }
            79 => {
                local_3 = (p1.call("getContraption", vec![]).call("writeNBT", vec![p1.call("registryAccess", vec![]), JvmVal::Int(0)])).clone();
                local_3.call("remove", vec![JvmVal::string("UUID")]);
                local_3.call("remove", vec![JvmVal::string("Pos")]);
                local_3.call("remove", vec![JvmVal::string("Motion")]);
                ctx.call_static("net/createmod/catnip/nbt/NBTHelper", "writeEnum", vec![local_3.clone(), JvmVal::string("InitialOrientation"), p1.call("getInitialOrientation", vec![])]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllDataComponents", "MINECRAFT_CONTRAPTION_DATA", vec![]);
                return local_2;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/contraptions/minecart/MinecartCouplingItem.class :: handleInteractionWithMinecart (java static)
pub fn handle_interaction_with_minecart(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getTarget", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_1, "net/minecraft/world/entity/vehicle/AbstractMinecart"), &JvmVal::Int(0)).as_i32() == 0 { pc = 20; continue 'jvm; }
                pc = 13;
            }
            12 => {
                local_2 = (local_1).clone();
                pc = 21;
            }
            20 => {
                return;
            }
            21 => {
                local_3 = (p0.call("getEntity", vec![])).clone();
                if !jvm_is_null(&local_3) { pc = 31; continue 'jvm; }
                pc = 31;
            }
            30 => {
                return;
            }
            31 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllAttachmentTypes", "MINECART_CONTROLLER", vec![]);
                local_4 = (local_2.call("getData", vec![arg_tmp_0.clone()])).clone();
                if jvm_ref_eq(&local_4, &ctx.call_static("com/simibubi/create/content/contraptions/minecart/capability/MinecartController", "EMPTY", vec![])) { pc = 59; continue 'jvm; }
                pc = 53;
            }
            51 => {
                if jvm_cmp(&local_4.call("isPresent", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 60; continue 'jvm; }
                pc = 60;
            }
            59 => {
                return;
            }
            60 => {
                local_5 = (local_3.call("getItemInHand", vec![p0.call("getHand", vec![])])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "MINECART_COUPLING", vec![]).call("isIn", vec![local_5.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 93; continue 'jvm; }
                pc = 82;
            }
            81 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/contraptions/minecart/MinecartCouplingItem", "onCouplingInteractOnMinecart", vec![p0.clone(), local_2.clone(), local_3.clone(), local_4.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 117; continue 'jvm; }
                pc = 93;
            }
            92 => {
                return;
            }
            93 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WRENCH", vec![]).call("isIn", vec![local_5.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 116; continue 'jvm; }
                pc = 105;
            }
            104 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/content/contraptions/minecart/MinecartCouplingItem", "onWrenchInteractOnMinecart", vec![p0.clone(), local_2.clone(), local_3.clone(), local_4.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 117; continue 'jvm; }
                pc = 116;
            }
            115 => {
                return;
            }
            116 => {
                return;
            }
            117 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/contraptions/minecart/MinecartCouplingItem.class :: onCouplingInteractOnMinecart (java static)
pub fn on_coupling_interact_on_minecart(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (p0.call("getLevel", vec![])).clone();
                if jvm_cmp(&p3.call("isFullyCoupled", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 29; continue 'jvm; }
                pc = 15;
            }
            13 => {
                if jvm_cmp(&local_4.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() != 0 { pc = 27; continue 'jvm; }
                pc = 22;
            }
            21 => {
                ctx.call_static("com/simibubi/create/content/contraptions/minecart/CouplingHandler", "status", vec![p2.clone(), JvmVal::string("two_couplings_max")]);
                pc = 28;
            }
            27 => {
                return JvmVal::Int(1);
            }
            29 => {
                if jvm_is_null(&local_4) { pc = 57; continue 'jvm; }
                pc = 36;
            }
            34 => {
                if jvm_cmp(&local_4.get_field("isClientSide"), &JvmVal::Int(0)).as_i32() == 0 { pc = 57; continue 'jvm; }
                pc = 45;
            }
            42 => {
                p1.call("executeOnClientOnly", vec![JvmVal::Null]);
                pc = 58;
            }
            57 => {
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/contraptions/minecart/MinecartCouplingItem.class :: onWrenchInteractOnMinecart (java static)
pub fn on_wrench_interact_on_minecart(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p3.call("isConnectedToCoupling", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 11; continue 'jvm; }
                pc = 8;
            }
            7 => {
                pc = 12;
            }
            11 => {
                pc = 13;
            }
            12 => {
                if jvm_cmp(&p3.call("isLeadingCoupling", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 23; continue 'jvm; }
                pc = 20;
            }
            19 => {
                pc = 24;
            }
            23 => {
                pc = 25;
            }
            24 => {
                local_4 = (jvm_bin("+", &JvmVal::Int(1), &JvmVal::Int(0))).clone();
                if jvm_cmp(&local_4, &JvmVal::Int(0)).as_i32() != 0 { pc = 34; continue 'jvm; }
                pc = 33;
            }
            32 => {
                return JvmVal::Int(0);
            }
            34 => {
                if jvm_cmp(&p0.call("getLevel", vec![]).get_field("isClientSide"), &JvmVal::Int(0)).as_i32() == 0 { pc = 46; continue 'jvm; }
                pc = 45;
            }
            44 => {
                return JvmVal::Int(1);
            }
            46 => {
                local_5 = (ctx.call_static("net/createmod/catnip/data/Iterate", "trueAndFalse", vec![])).clone();
                local_6 = (jvm_array_length(&local_5)).clone();
                local_7 = (JvmVal::Int(0)).clone();
                pc = 61;
            }
            59 => {
                if jvm_cmp(&local_7, &local_6).as_i32() >= 0 { pc = 91; continue 'jvm; }
                pc = 68;
            }
            66 => {
                local_8 = (jvm_array_load(&local_5, local_7.as_i32())).clone();
                if jvm_cmp(&p3.call("hasContraptionCoupling", vec![local_8.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 85; continue 'jvm; }
                pc = 85;
            }
            82 => {
                local_4 = jvm_iinc(&local_4, -1);
                pc = 88;
            }
            85 => {
                local_7 = jvm_iinc(&local_7, 1);
                pc = 59;
            }
            91 => {
                ctx.call_static("com/simibubi/create/content/contraptions/minecart/CouplingHandler", "status", vec![p2.clone(), JvmVal::string("removed")]);
                p3.call("decouple", vec![]);
                if jvm_cmp(&p2.call("isCreative", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 133; continue 'jvm; }
                pc = 109;
            }
            108 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/AllItems", "MINECART_COUPLING", vec![]).call("get", vec![]);
                p2.call("getInventory", vec![]).call("placeItemBackInInventory", vec![JvmVal::obj("net/minecraft/world/item/ItemStack")]);
                pc = 134;
            }
            133 => {
                return JvmVal::Int(1);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/contraptions/minecart/MinecartCouplingItem.class :: cartClicked (java static)
pub fn cart_clicked(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/content/contraptions/minecart/CouplingHandlerClient", "onCartClicked", vec![p0.clone(), p1.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/contraptions/minecart/MinecartCouplingItem.class :: lambda$onCouplingInteractOnMinecart$1 (java static)
pub fn lambda_on_coupling_interact_on_minecart_1(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Null;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/contraptions/minecart/MinecartCouplingItem.class :: lambda$onCouplingInteractOnMinecart$0 (java static)
pub fn lambda_on_coupling_interact_on_minecart_0(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/content/contraptions/minecart/MinecartCouplingItem", "cartClicked", vec![p0.clone(), p1.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/contraptions/glue/SuperGlueItem.class :: glueItemAlwaysPlacesWhenUsed (java static)
pub fn glue_item_always_places_when_used(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_is_null(&p0.call("getHitVec", vec![])) { pc = 52; continue 'jvm; }
                pc = 8;
            }
            7 => {
                local_1 = (p0.call("getLevel", vec![]).call("getBlockState", vec![p0.call("getHitVec", vec![]).call("getBlockPos", vec![])])).clone();
                local_3 = (local_1.call("getBlock", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_3, "com/simibubi/create/content/contraptions/chassis/AbstractChassisBlock"), &JvmVal::Int(0)).as_i32() == 0 { pc = 52; continue 'jvm; }
                pc = 35;
            }
            34 => {
                local_2 = (local_3).clone();
                if jvm_is_null(&local_2.call("getGlueableSide", vec![local_1.clone(), p0.call("getFace", vec![])])) { pc = 52; continue 'jvm; }
                pc = 52;
            }
            51 => {
                return;
            }
            52 => {
                if jvm_cmp(&jvm_instanceof(&p0.call("getItemStack", vec![]).call("getItem", vec![]), "com/simibubi/create/content/contraptions/glue/SuperGlueItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 72; continue 'jvm; }
                pc = 66;
            }
            65 => {
    let arg_tmp_0 = ctx.call_static("net/neoforged/neoforge/common/util/TriState", "FALSE", vec![]);
                p0.call("setUseBlock", vec![arg_tmp_0.clone()]);
                pc = 72;
            }
            72 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/content/contraptions/glue/SuperGlueItem.class :: canAttackBlock (java instance)
pub fn can_attack_block(ctx: &mut BridgeCtx, p_this: JvmVal, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) -> JvmVal {
    let mut p_this: JvmVal = p_this.clone();
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/content/contraptions/glue/SuperGlueItem.class :: spawnParticles (java static)
pub fn spawn_particles(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (ctx.call_static("net/minecraft/world/phys/Vec3", "atLowerCornerOf", vec![p2.call("getNormal", vec![])])).clone();
                local_5 = (ctx.call_static("net/createmod/catnip/math/VecHelper", "axisAlingedPlaneOf", vec![local_4.clone()])).clone();
                local_6 = (ctx.call_static("net/createmod/catnip/math/VecHelper", "getCenterOf", vec![p1.clone()]).call("add", vec![local_4.call("scale", vec![JvmVal::Double(0.5_f64)])])).clone();
                if jvm_cmp(&p3, &JvmVal::Int(0)).as_i32() == 0 { pc = 41; continue 'jvm; }
                pc = 38;
            }
            37 => {
                pc = 59;
            }
            41 => {
                pc = 61;
            }
            59 => {
                local_7 = (jvm_bin("+", &JvmVal::Float(0.25_f32), &jvm_bin("*", &JvmVal::Float(0.25_f32), &jvm_bin("-", &p0.get_field("random").call("nextFloat", vec![]), &JvmVal::Float(0.5_f32))))).clone();
                local_5 = (local_5.call("scale", vec![jvm_conv(&local_7, "D")])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/item/Items", "SLIME_BALL", vec![]);
                local_8 = (JvmVal::obj("net/minecraft/world/item/ItemStack")).clone();
                if jvm_cmp(&p3, &JvmVal::Int(0)).as_i32() == 0 { pc = 92; continue 'jvm; }
                pc = 89;
            }
            87 => {
                pc = 94;
            }
            92 => {
                pc = 96;
            }
            94 => {
                local_9 = (JvmVal::Int(15)).clone();
                pc = 98;
            }
            96 => {
                if jvm_cmp(&local_9, &JvmVal::Int(0)).as_i32() <= 0 { pc = 254; continue 'jvm; }
                pc = 103;
            }
            101 => {
                local_10 = (ctx.call_static("net/createmod/catnip/math/VecHelper", "rotate", vec![local_5.clone(), jvm_conv(&jvm_bin("*", &JvmVal::Float(360_f32), &p0.get_field("random").call("nextFloat", vec![])), "D"), p2.call("getAxis", vec![])])).clone();
                local_11 = (local_10.call("normalize", vec![]).call("scale", vec![JvmVal::Double(0.0625_f64)])).clone();
                if jvm_cmp(&p3, &JvmVal::Int(0)).as_i32() == 0 { pc = 193; continue 'jvm; }
                pc = 145;
            }
            142 => {
    let arg_tmp_0 = ctx.call_static("net/minecraft/util/Mth", "clamp", vec![local_10.get_field("z"), JvmVal::Double(-0.5_f64), JvmVal::Double(0.5_f64)]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/util/Mth", "clamp", vec![local_10.get_field("y"), JvmVal::Double(-0.5_f64), JvmVal::Double(0.5_f64)]);
    let arg_tmp_2 = ctx.call_static("net/minecraft/util/Mth", "clamp", vec![local_10.get_field("x"), JvmVal::Double(-0.5_f64), JvmVal::Double(0.5_f64)]);
                local_10 = (JvmVal::obj("net/minecraft/world/phys/Vec3")).clone();
                pc = 195;
            }
            193 => {
                local_12 = (local_6.call("add", vec![local_10.clone()])).clone();
    let arg_tmp_0 = ctx.call_static("net/minecraft/core/particles/ParticleTypes", "ITEM", vec![]);
                p0.call("addParticle", vec![JvmVal::obj("net/minecraft/core/particles/ItemParticleOption"), local_12.get_field("x"), local_12.get_field("y"), local_12.get_field("z"), local_11.get_field("x"), local_11.get_field("y"), local_11.get_field("z")]);
                local_9 = jvm_iinc(&local_9, -1);
                pc = 96;
            }
            254 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: onServerTick (java static)
pub fn on_server_tick(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/Create", "SCHEMATIC_RECEIVER", vec![]).call("tick", vec![]);
                ctx.call_static("com/simibubi/create/Create", "LAGGER", vec![]).call("tick", vec![]);
                ctx.call_static("com/simibubi/create/foundation/utility/ServerSpeedProvider", "serverTick", vec![]);
                ctx.call_static("com/simibubi/create/Create", "RAILWAYS", vec![]).get_field("sync").call("serverTick", vec![]);
                ctx.call_static("com/simibubi/create/compat/trainmap/TrainMapSync", "serverTick", vec![p0.clone()]);
                ctx.call_static("com/simibubi/create/content/kinetics/chainConveyor/ServerChainConveyorHandler", "tick", vec![]);
                ctx.call_static("com/simibubi/create/foundation/utility/TickBasedCache", "tick", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: onChunkUnloaded (java static)
pub fn on_chunk_unloaded(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/content/contraptions/minecart/capability/CapabilityMinecartController", "onChunkUnloaded", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: playerLoggedIn (java static)
pub fn player_logged_in(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getEntity", vec![])).clone();
                ctx.call_static("com/simibubi/create/content/equipment/toolbox/ToolboxHandler", "playerLogin", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/Create", "RAILWAYS", vec![]).call("playerLogin", vec![local_1.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: playerLoggedOut (java static)
pub fn player_logged_out(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getEntity", vec![])).clone();
                ctx.call_static("com/simibubi/create/Create", "RAILWAYS", vec![]).call("playerLogout", vec![local_1.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: onServerWorldTick (java static)
pub fn on_server_world_tick(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getLevel", vec![])).clone();
                if jvm_cmp(&local_1.call("isClientSide", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 13; continue 'jvm; }
                pc = 13;
            }
            12 => {
                return;
            }
            13 => {
                ctx.call_static("com/simibubi/create/content/contraptions/ContraptionHandler", "tick", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/content/contraptions/minecart/capability/CapabilityMinecartController", "tick", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/content/contraptions/minecart/CouplingPhysics", "tick", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/content/redstone/link/controller/LinkedControllerServerHandler", "tick", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/content/contraptions/actors/trainControls/ControlsServerHandler", "tick", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/Create", "RAILWAYS", vec![]).call("tick", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/Create", "LOGISTICS", vec![]).call("tick", vec![local_1.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: onEntityTick (java static)
pub fn on_entity_tick(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/content/contraptions/minecart/capability/CapabilityMinecartController", "entityTick", vec![p0.clone()]);
                local_2 = (p0.call("getEntity", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_2, "net/minecraft/world/entity/LivingEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 36; continue 'jvm; }
                pc = 17;
            }
            16 => {
                local_1 = (local_2).clone();
                local_2 = (local_1.call("level", vec![])).clone();
                ctx.call_static("com/simibubi/create/content/contraptions/ContraptionHandler", "entitiesWhoJustDismountedGetSentToTheRightLocation", vec![local_1.clone(), local_2.clone()]);
                ctx.call_static("com/simibubi/create/content/equipment/toolbox/ToolboxHandler", "entityTick", vec![local_1.clone(), local_2.clone()]);
                pc = 36;
            }
            36 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: onEntityAdded (java static)
pub fn on_entity_added(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getEntity", vec![])).clone();
                local_2 = (p0.call("getLevel", vec![])).clone();
                ctx.call_static("com/simibubi/create/content/contraptions/ContraptionHandler", "addSpawnedContraptionsToCollisionList", vec![local_1.clone(), local_2.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: onEntityAttackedByPlayer (java static)
pub fn on_entity_attacked_by_player(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/content/equipment/wrench/WrenchItem", "wrenchInstaKillsMinecarts", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: registerCommands (java static)
pub fn register_commands(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/infrastructure/command/AllCommands", "register", vec![p0.call("getDispatcher", vec![])]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: onEntityEnterSection (java static)
pub fn on_entity_enter_section(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/content/trains/entity/CarriageEntityHandler", "onEntityEnterSection", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: addReloadListeners (java static)
pub fn add_reload_listeners(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/recipe/RecipeFinder", "LISTENER", vec![]);
                p0.call("addListener", vec![arg_tmp_0.clone()]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/recipe/trie/RecipeTrieFinder", "LISTENER", vec![]);
                p0.call("addListener", vec![arg_tmp_0.clone()]);
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/content/kinetics/belt/BeltHelper", "LISTENER", vec![]);
                p0.call("addListener", vec![arg_tmp_0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: serverStopping (java static)
pub fn server_stopping(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/Create", "SCHEMATIC_RECEIVER", vec![]).call("shutdown", vec![]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: onLoadWorld (java static)
pub fn on_load_world(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getLevel", vec![])).clone();
                ctx.call_static("com/simibubi/create/Create", "REDSTONE_LINK_NETWORK_HANDLER", vec![]).call("onLoadWorld", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/Create", "TORQUE_PROPAGATOR", vec![]).call("onLoadWorld", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/Create", "RAILWAYS", vec![]).call("levelLoaded", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/Create", "LOGISTICS", vec![]).call("levelLoaded", vec![local_1.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: onUnloadWorld (java static)
pub fn on_unload_world(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getLevel", vec![])).clone();
                ctx.call_static("com/simibubi/create/Create", "REDSTONE_LINK_NETWORK_HANDLER", vec![]).call("onUnloadWorld", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/Create", "TORQUE_PROPAGATOR", vec![]).call("onUnloadWorld", vec![local_1.clone()]);
                ctx.call_static("net/createmod/catnip/data/WorldAttached", "invalidateWorld", vec![local_1.clone()]);
                ctx.call_static("com/simibubi/create/content/kinetics/drill/CobbleGenOptimisation", "invalidateWorld", vec![local_1.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: attachData (java static)
pub fn attach_data(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/content/contraptions/minecart/capability/CapabilityMinecartController", "attach", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: onEntityLeaveLevel (java static)
pub fn on_entity_leave_level(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&p0.call("getEntity", vec![]).call("isAlive", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 14; continue 'jvm; }
                pc = 11;
            }
            10 => {
                ctx.call_static("com/simibubi/create/content/contraptions/minecart/capability/CapabilityMinecartController", "onEntityDeath", vec![p0.clone()]);
                pc = 14;
            }
            14 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: startTracking (java static)
pub fn start_tracking(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/content/contraptions/minecart/capability/CapabilityMinecartController", "startTracking", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/events/CommonEvents.class :: leftClickEmpty (java static)
pub fn left_click_empty(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getMainHandItem", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_1.call("getItem", vec![]), "com/simibubi/create/content/equipment/zapper/ZapperItem"), &JvmVal::Int(0)).as_i32() == 0 { pc = 21; continue 'jvm; }
                pc = 16;
            }
            15 => {
                pc = 21;
            }
            21 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/blockEntity/behaviour/edgeInteraction/EdgeInteractionHandler.class :: onBlockActivated (java static)
pub fn on_block_activated_3(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getLevel", vec![])).clone();
                local_2 = (p0.call("getPos", vec![])).clone();
                local_3 = (p0.call("getEntity", vec![])).clone();
                local_4 = (p0.call("getHand", vec![])).clone();
                local_5 = (local_3.call("getItemInHand", vec![local_4.clone()])).clone();
                if jvm_cmp(&local_3.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 43; continue 'jvm; }
                pc = 37;
            }
            36 => {
                if jvm_cmp(&local_3.call("isSpectator", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 44; continue 'jvm; }
                pc = 44;
            }
            43 => {
                return;
            }
            44 => {
    let arg_tmp_0 = ctx.call_static("com/simibubi/create/foundation/blockEntity/behaviour/edgeInteraction/EdgeInteractionBehaviour", "TYPE", vec![]);
                local_6 = (ctx.call_static("com/simibubi/create/foundation/blockEntity/behaviour/BlockEntityBehaviour", "get", vec![local_1.clone(), local_2.clone(), arg_tmp_0.clone()])).clone();
                if !jvm_is_null(&local_6) { pc = 63; continue 'jvm; }
                pc = 63;
            }
            62 => {
                return;
            }
            63 => {
                if jvm_cmp(&local_6.get_field("requiredItem").call("test", vec![local_5.call("getItem", vec![])]), &JvmVal::Int(0)).as_i32() != 0 { pc = 82; continue 'jvm; }
                pc = 82;
            }
            81 => {
                return;
            }
            82 => {
                local_7 = (ctx.call_static("com/simibubi/create/foundation/utility/RaycastHelper", "rayTraceRange", vec![local_1.clone(), local_3.clone(), JvmVal::Double(10_f64)])).clone();
                if !jvm_is_null(&local_7) { pc = 98; continue 'jvm; }
                pc = 98;
            }
            97 => {
                return;
            }
            98 => {
                local_8 = (ctx.call_static("com/simibubi/create/foundation/blockEntity/behaviour/edgeInteraction/EdgeInteractionHandler", "getActivatedDirection", vec![local_1.clone(), local_2.clone(), local_7.call("getDirection", vec![]), local_7.call("getLocation", vec![]), local_6.clone()])).clone();
                if !jvm_is_null(&local_8) { pc = 123; continue 'jvm; }
                pc = 123;
            }
            122 => {
                return;
            }
            123 => {
                if jvm_ref_eq(&p0.call("getSide", vec![]), &ctx.call_static("net/neoforged/fml/LogicalSide", "CLIENT", vec![])) { pc = 151; continue 'jvm; }
                pc = 135;
            }
            133 => {
                local_6.get_field("connectionCallback").call("apply", vec![local_1.clone(), local_2.clone(), local_2.call("relative", vec![local_8.clone()])]);
                pc = 152;
            }
            151 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/sounds/SoundSource", "BLOCKS", vec![]);
    let arg_tmp_1 = ctx.call_static("net/minecraft/sounds/SoundEvents", "ITEM_FRAME_ADD_ITEM", vec![]);
                local_1.call("playSound", vec![JvmVal::Null, local_2.clone(), arg_tmp_1.clone(), arg_tmp_0.clone(), JvmVal::Float(0.25_f32), JvmVal::Float(0.1_f32)]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/blockEntity/behaviour/edgeInteraction/EdgeInteractionHandler.class :: getConnectiveSides (java static)
pub fn get_connective_sides(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_4 = (JvmVal::obj("java/util/ArrayList")).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/foundation/utility/BlockHelper", "hasBlockSolidSide", vec![p0.call("getBlockState", vec![p1.call("relative", vec![p2.clone()])]), p0.clone(), p1.call("relative", vec![p2.clone()]), p2.call("getOpposite", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 39; continue 'jvm; }
                pc = 38;
            }
            36 => {
                return local_4;
            }
            39 => {
                local_5 = (ctx.call_static("net/createmod/catnip/data/Iterate", "directions", vec![])).clone();
                local_6 = (jvm_array_length(&local_5)).clone();
                local_7 = (JvmVal::Int(0)).clone();
                pc = 54;
            }
            52 => {
                if jvm_cmp(&local_7, &local_6).as_i32() >= 0 { pc = 155; continue 'jvm; }
                pc = 61;
            }
            59 => {
                local_8 = (jvm_array_load(&local_5, local_7.as_i32())).clone();
                if !jvm_ref_eq(&local_8.call("getAxis", vec![]), &p2.call("getAxis", vec![])) { pc = 81; continue 'jvm; }
                pc = 81;
            }
            78 => {
                pc = 149;
            }
            81 => {
                local_9 = (p1.call("relative", vec![local_8.clone()])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/foundation/utility/BlockHelper", "hasBlockSolidSide", vec![p0.call("getBlockState", vec![local_9.call("relative", vec![p2.clone()])]), p0.clone(), local_9.call("relative", vec![p2.clone()]), p2.call("getOpposite", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 119; continue 'jvm; }
                pc = 119;
            }
            116 => {
                pc = 149;
            }
            119 => {
                if jvm_cmp(&p3.get_field("connectivityPredicate").call("test", vec![p0.clone(), p1.clone(), p2.clone(), local_8.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 139; continue 'jvm; }
                pc = 139;
            }
            136 => {
                pc = 149;
            }
            139 => {
                pc = 152;
            }
            149 => {
                local_7 = jvm_iinc(&local_7, 1);
                pc = 52;
            }
            155 => {
                return local_4;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/blockEntity/behaviour/edgeInteraction/EdgeInteractionHandler.class :: getActivatedDirection (java static)
pub fn get_activated_direction(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal, p4: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut p4: JvmVal = p4.clone();
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_5 = (ctx.call_static("com/simibubi/create/foundation/blockEntity/behaviour/edgeInteraction/EdgeInteractionHandler", "getConnectiveSides", vec![p0.clone(), p1.clone(), p2.clone(), p4.clone()]).call("iterator", vec![])).clone();
                pc = 17;
            }
            15 => {
                if jvm_cmp(&local_5.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 60; continue 'jvm; }
                pc = 27;
            }
            25 => {
                local_6 = (local_5.call("next", vec![])).clone();
                local_7 = (ctx.call_static("com/simibubi/create/foundation/blockEntity/behaviour/edgeInteraction/EdgeInteractionHandler", "getBB", vec![p1.clone(), local_6.clone()])).clone();
                if jvm_cmp(&local_7.call("contains", vec![p3.clone()]), &JvmVal::Int(0)).as_i32() == 0 { pc = 57; continue 'jvm; }
                pc = 56;
            }
            54 => {
                return local_6;
            }
            57 => {
                pc = 15;
            }
            60 => {
                return JvmVal::Null;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/blockEntity/behaviour/edgeInteraction/EdgeInteractionHandler.class :: getBB (java static)
pub fn get_bb(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut local_12: JvmVal = JvmVal::Null;
    let mut local_13: JvmVal = JvmVal::Null;
    let mut local_14: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_2 = (JvmVal::obj("net/minecraft/world/phys/AABB")).clone();
                local_3 = (p1.call("getNormal", vec![])).clone();
                local_4 = (local_3.call("getX", vec![])).clone();
                local_5 = (local_3.call("getY", vec![])).clone();
                local_6 = (local_3.call("getZ", vec![])).clone();
                local_7 = (JvmVal::Double(0.625_f64)).clone();
                local_9 = (jvm_bin("*", &jvm_conv(&ctx.call_static("java/lang/Math", "abs", vec![local_4.clone()]), "D"), &local_7)).clone();
                local_11 = (jvm_bin("*", &jvm_conv(&ctx.call_static("java/lang/Math", "abs", vec![local_5.clone()]), "D"), &local_7)).clone();
                local_13 = (jvm_bin("*", &jvm_conv(&ctx.call_static("java/lang/Math", "abs", vec![local_6.clone()]), "D"), &local_7)).clone();
                local_2 = (local_2.call("contract", vec![local_9.clone(), local_11.clone(), local_13.clone()])).clone();
                local_2 = (local_2.call("move", vec![jvm_bin("/", &local_9, &JvmVal::Double(2_f64)), jvm_bin("/", &local_11, &JvmVal::Double(2_f64)), jvm_bin("/", &local_13, &JvmVal::Double(2_f64))])).clone();
                local_2 = (local_2.call("move", vec![jvm_bin("/", &jvm_conv(&local_4, "D"), &JvmVal::Double(2_f64)), jvm_bin("/", &jvm_conv(&local_5, "D"), &JvmVal::Double(2_f64)), jvm_bin("/", &jvm_conv(&local_6, "D"), &JvmVal::Double(2_f64))])).clone();
                local_2 = (local_2.call("inflate", vec![JvmVal::Double(0.00390625_f64)])).clone();
                return local_2;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/blockEntity/behaviour/ValueSettingsInputHandler.class :: onBlockActivated (java static)
pub fn on_block_activated_4(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut local_10: JvmVal = JvmVal::Null;
    let mut local_11: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                local_1 = (p0.call("getLevel", vec![])).clone();
                local_2 = (p0.call("getPos", vec![])).clone();
                local_3 = (p0.call("getEntity", vec![])).clone();
                local_4 = (p0.call("getHand", vec![])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/foundation/blockEntity/behaviour/ValueSettingsInputHandler", "canInteract", vec![local_3.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 29; continue 'jvm; }
                pc = 29;
            }
            28 => {
                return;
            }
            29 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllBlocks", "CLIPBOARD", vec![]).call("isIn", vec![local_3.call("getMainHandItem", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 43; continue 'jvm; }
                pc = 43;
            }
            42 => {
                return;
            }
            43 => {
                local_6 = (local_1.call("getBlockEntity", vec![local_2.clone()])).clone();
                if jvm_cmp(&jvm_instanceof(&local_6, "com/simibubi/create/foundation/blockEntity/SmartBlockEntity"), &JvmVal::Int(0)).as_i32() == 0 { pc = 68; continue 'jvm; }
                pc = 60;
            }
            58 => {
                local_5 = (local_6).clone();
                pc = 69;
            }
            68 => {
                return;
            }
            69 => {
                if !jvm_ref_eq(&p0.call("getSide", vec![]), &ctx.call_static("net/neoforged/fml/LogicalSide", "CLIENT", vec![])) { pc = 93; continue 'jvm; }
                pc = 82;
            }
            79 => {
                p0.call("executeOnClientOnly", vec![JvmVal::Null]);
                pc = 94;
            }
            93 => {
                if jvm_cmp(&p0.call("isCanceled", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 101; continue 'jvm; }
                pc = 101;
            }
            100 => {
                return;
            }
            101 => {
                local_6 = (local_5.call("getAllBehaviours", vec![]).call("iterator", vec![])).clone();
                pc = 115;
            }
            113 => {
                if jvm_cmp(&local_6.call("hasNext", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 433; continue 'jvm; }
                pc = 125;
            }
            123 => {
                local_7 = (local_6.call("next", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_7, "com/simibubi/create/foundation/blockEntity/behaviour/ValueSettingsBehaviour"), &JvmVal::Int(0)).as_i32() == 0 { pc = 113; continue 'jvm; }
                pc = 145;
            }
            143 => {
                local_8 = (local_7).clone();
                if jvm_cmp(&local_8.call("bypassesInput", vec![local_3.call("getMainHandItem", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 167; continue 'jvm; }
                pc = 167;
            }
            164 => {
                pc = 113;
            }
            167 => {
                if jvm_cmp(&local_8.call("mayInteract", vec![local_3.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 181; continue 'jvm; }
                pc = 181;
            }
            178 => {
                pc = 113;
            }
            181 => {
                local_9 = (p0.call("getHitVec", vec![])).clone();
                if !jvm_is_null(&local_9) { pc = 193; continue 'jvm; }
                pc = 193;
            }
            192 => {
                return;
            }
            193 => {
                if jvm_cmp(&jvm_instanceof(&local_7, "com/simibubi/create/foundation/blockEntity/behaviour/filtering/SidedFilteringBehaviour"), &JvmVal::Int(0)).as_i32() == 0 { pc = 224; continue 'jvm; }
                pc = 203;
            }
            201 => {
                local_7 = (local_7.call("get", vec![local_9.call("getDirection", vec![])])).clone();
                if !jvm_is_null(&local_7) { pc = 224; continue 'jvm; }
                pc = 224;
            }
            221 => {
                pc = 113;
            }
            224 => {
                if jvm_cmp(&local_8.call("isActive", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 237; continue 'jvm; }
                pc = 237;
            }
            234 => {
                pc = 113;
            }
            237 => {
                if jvm_cmp(&local_8.call("onlyVisibleWithWrench", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 265; continue 'jvm; }
                pc = 248;
            }
            247 => {
    let arg_tmp_0 = ctx.call_static("net/neoforged/neoforge/common/Tags$Items", "TOOLS_WRENCH", vec![]);
                if jvm_cmp(&local_3.call("getItemInHand", vec![local_4.clone()]).call("is", vec![arg_tmp_0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 265; continue 'jvm; }
                pc = 265;
            }
            262 => {
                pc = 113;
            }
            265 => {
                local_11 = (local_8.call("getSlotPositioning", vec![])).clone();
                if jvm_cmp(&jvm_instanceof(&local_11, "com/simibubi/create/foundation/blockEntity/behaviour/ValueBoxTransform$Sided"), &JvmVal::Int(0)).as_i32() == 0 { pc = 321; continue 'jvm; }
                pc = 284;
            }
            282 => {
                local_10 = (local_11).clone();
                if jvm_cmp(&local_10.call("isSideActive", vec![local_5.call("getBlockState", vec![]), local_9.call("getDirection", vec![])]), &JvmVal::Int(0)).as_i32() != 0 { pc = 310; continue 'jvm; }
                pc = 310;
            }
            307 => {
                pc = 113;
            }
            310 => {
                pc = 322;
            }
            321 => {
                local_10 = (jvm_instanceof(&local_3, "net/neoforged/neoforge/common/util/FakePlayer")).clone();
                if jvm_cmp(&local_8.call("testHit", vec![local_9.call("getLocation", vec![])]), &JvmVal::Int(0)).as_i32() != 0 { pc = 350; continue 'jvm; }
                pc = 344;
            }
            342 => {
                if jvm_cmp(&local_10, &JvmVal::Int(0)).as_i32() != 0 { pc = 350; continue 'jvm; }
                pc = 350;
            }
            347 => {
                pc = 113;
            }
            350 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
    let arg_tmp_0 = ctx.call_static("net/minecraft/world/InteractionResult", "SUCCESS", vec![]);
                p0.call("setCancellationResult", vec![arg_tmp_0.clone()]);
                if jvm_cmp(&local_8.call("acceptsValueSettings", vec![]), &JvmVal::Int(0)).as_i32() == 0 { pc = 377; continue 'jvm; }
                pc = 374;
            }
            372 => {
                if jvm_cmp(&local_10, &JvmVal::Int(0)).as_i32() == 0 { pc = 395; continue 'jvm; }
                pc = 379;
            }
            377 => {
                local_8.call("onShortInteract", vec![local_3.clone(), local_4.clone(), local_9.call("getDirection", vec![]), local_9.clone()]);
                return;
            }
            395 => {
                if !jvm_ref_eq(&p0.call("getSide", vec![]), &ctx.call_static("net/neoforged/fml/LogicalSide", "CLIENT", vec![])) { pc = 432; continue 'jvm; }
                pc = 407;
            }
            405 => {
                local_11 = (local_7.call("getType", vec![])).clone();
                local_9.call("executeOnClientOnly", vec![JvmVal::Null]);
                pc = 433;
            }
            432 => {
                return;
            }
            433 => {
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/blockEntity/behaviour/ValueSettingsInputHandler.class :: canInteract (java static)
pub fn can_interact(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_is_null(&p0) { pc = 29; continue 'jvm; }
                pc = 5;
            }
            4 => {
                if jvm_cmp(&p0.call("isSpectator", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 29; continue 'jvm; }
                pc = 12;
            }
            11 => {
                if jvm_cmp(&p0.call("isShiftKeyDown", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 29; continue 'jvm; }
                pc = 19;
            }
            18 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/foundation/utility/AdventureUtil", "isAdventure", vec![p0.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 29; continue 'jvm; }
                pc = 26;
            }
            25 => {
                pc = 30;
            }
            29 => {
                pc = 30;
            }
            30 => {
                return JvmVal::Int(0);
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/blockEntity/behaviour/ValueSettingsInputHandler.class :: lambda$onBlockActivated$3 (java static)
pub fn lambda_on_block_activated_3(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Null;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/blockEntity/behaviour/ValueSettingsInputHandler.class :: lambda$onBlockActivated$2 (java static)
pub fn lambda_on_block_activated_2(ctx: &mut BridgeCtx, p0: JvmVal, p1: JvmVal, p2: JvmVal, p3: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut p1: JvmVal = p1.clone();
    let mut p2: JvmVal = p2.clone();
    let mut p3: JvmVal = p3.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/CreateClient", "VALUE_SETTINGS_HANDLER", vec![]).call("startInteractionWith", vec![p0.clone(), p1.clone(), p2.clone(), p3.call("getDirection", vec![])]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/blockEntity/behaviour/ValueSettingsInputHandler.class :: lambda$onBlockActivated$1 (java static)
pub fn lambda_on_block_activated_1(ctx: &mut BridgeCtx, p0: JvmVal) -> JvmVal {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                return JvmVal::Null;
            }
                _ => { break 'jvm; }
            }
        }
        JvmVal::Null // fell off the end of a value-returning method
    }


// --- from com/simibubi/create/foundation/blockEntity/behaviour/ValueSettingsInputHandler.class :: lambda$onBlockActivated$0 (java static)
pub fn lambda_on_block_activated_0(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                ctx.call_static("com/simibubi/create/CreateClient", "VALUE_SETTINGS_HANDLER", vec![]).call("cancelIfWarmupAlreadyStarted", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/block/ItemUseOverrides.class :: addBlock (java static)
pub fn add_block(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
    let arg_tmp_0 = ctx.call_static("net/createmod/catnip/registry/RegisteredObjectsHelper", "getKeyOrThrow", vec![p0.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- from com/simibubi/create/foundation/block/ItemUseOverrides.class :: onBlockActivated (java static)
pub fn on_block_activated_5(ctx: &mut BridgeCtx, p0: JvmVal) {
    let mut p0: JvmVal = p0.clone();
    let mut local_1: JvmVal = JvmVal::Null;
    let mut local_2: JvmVal = JvmVal::Null;
    let mut local_3: JvmVal = JvmVal::Null;
    let mut local_4: JvmVal = JvmVal::Null;
    let mut local_5: JvmVal = JvmVal::Null;
    let mut local_6: JvmVal = JvmVal::Null;
    let mut local_7: JvmVal = JvmVal::Null;
    let mut local_8: JvmVal = JvmVal::Null;
    let mut local_9: JvmVal = JvmVal::Null;
    let mut pc: u32 = 0;
    'jvm: loop {
        match pc {
            0 => {
                if jvm_cmp(&ctx.call_static("com/simibubi/create/AllItems", "WRENCH", vec![]).call("isIn", vec![p0.call("getItemStack", vec![])]), &JvmVal::Int(0)).as_i32() == 0 { pc = 14; continue 'jvm; }
                pc = 14;
            }
            13 => {
                return;
            }
            14 => {
                local_1 = (p0.call("getLevel", vec![])).clone();
                local_2 = (p0.call("getPos", vec![])).clone();
                local_3 = (p0.call("getFace", vec![])).clone();
                local_4 = (p0.call("getEntity", vec![])).clone();
                local_5 = (p0.call("getHand", vec![])).clone();
                local_6 = (local_1.call("getBlockState", vec![local_2.clone()])).clone();
                local_7 = (ctx.call_static("net/createmod/catnip/registry/RegisteredObjectsHelper", "getKeyOrThrow", vec![local_6.call("getBlock", vec![])])).clone();
                if jvm_cmp(&ctx.call_static("com/simibubi/create/foundation/block/ItemUseOverrides", "OVERRIDES", vec![]).call("contains", vec![local_7.clone()]), &JvmVal::Int(0)).as_i32() != 0 { pc = 72; continue 'jvm; }
                pc = 72;
            }
            71 => {
                return;
            }
            72 => {
    let arg_tmp_0 = ctx.call_static("net/createmod/catnip/math/VecHelper", "getCenterOf", vec![local_2.clone()]);
                local_8 = (JvmVal::obj("net/minecraft/world/phys/BlockHitResult")).clone();
                local_9 = (ctx.call_static("com/simibubi/create/foundation/utility/BlockHelper", "invokeUse", vec![local_6.clone(), local_1.clone(), local_4.clone(), local_5.clone(), local_8.clone()])).clone();
                if jvm_cmp(&local_9.call("consumesAction", vec![]), &JvmVal::Int(0)).as_i32() != 0 { pc = 111; continue 'jvm; }
                pc = 111;
            }
            110 => {
                return;
            }
            111 => {
                p0.call("setCanceled", vec![JvmVal::Int(1)]);
                p0.call("setCancellationResult", vec![local_9.clone()]);
                return;
            }
                _ => { break 'jvm; }
            }
        }
    }


// --- native hook handlers (real signatures from Horizon's registry) ---
fn ctx_dimension(_hook: &str) -> String {
    // Game hooks use the game's current dimension (empty = default).
    String::new()
}

// # j2h: the mod's entry in Options > Mods
pub fn hook_mods(game: &mut crate::game::Game<'_>, mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    // Options > Mods row for this mod.
    mods.push(crate::game::native_hooks::NativeMod {
        id: "create".into(),
        title: format!("Create {} (native j2h)", "6.0.10"),
        description: "Java Create 6.0.10, transpiled to native Rust by j2h".into(),
        keys: Vec::new(),
    });
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler::rightClickingBlocksSelectsThem (decoded)
pub fn hook_0(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = right_clicking_blocks_selects_them(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$LeftClickBlock → lambda com/simibubi/create/content/kinetics/mechanicalArm/ArmInteractionPointHandler::leftClickingBlocksDeselectsThem (decoded)
pub fn hook_1(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], state: horizon_world::registry::BlockStateId, canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("left_click_block"),
    );
    let _result = left_clicking_blocks_deselects_them(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/living/LivingChangeTargetEvent → lambda com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer::entitiesDontRetaliate (decoded)
pub fn hook_2(game: &mut crate::game::Game<'_>, entity: i32, kind: &str, pos: [f64; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("attack_entity"),
    );
    let _result = entities_dont_retaliate(&mut ctx, JvmVal::Null);
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/kinetics/deployer/ManualApplicationRecipe::manualApplicationRecipesApplyInWorld (decoded)
pub fn hook_3(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = manual_application_recipes_apply_in_world(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/kinetics/crank/ValveHandleBlock::onBlockActivated (decoded)
pub fn hook_4(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = on_block_activated(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/kinetics/chainConveyor/ChainConveyorConnectionHandler::onItemUsedOnBlock (decoded)
pub fn hook_5(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = on_item_used_on_block(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$LeftClickBlock → lambda com/simibubi/create/content/equipment/tool/CardboardSwordItem::cardboardSwordsMakeNoiseOnClick (decoded)
pub fn hook_6(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], state: horizon_world::registry::BlockStateId, canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("left_click_block"),
    );
    let _result = cardboard_swords_make_noise_on_click(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$LeftClickBlock → lambda com/simibubi/create/content/equipment/zapper/ZapperInteractionHandler::leftClickingBlocksWithTheZapperSelectsTheBlock (decoded)
pub fn hook_7(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], state: horizon_world::registry::BlockStateId, canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("left_click_block"),
    );
    let _result = left_clicking_blocks_with_the_zapper_selects_the_block(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/level/BlockEvent$BreakEvent → lambda com/simibubi/create/content/equipment/symmetryWand/SymmetryHandler::onBlockDestroyed (decoded)
pub fn hook_8(game: &mut crate::game::Game<'_>, pos: [i32; 3], state: horizon_world::registry::BlockStateId, canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("destroy_block"),
    );
    let _result = on_block_destroyed(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/equipment/wrench/WrenchEventHandler::useOwnWrenchLogicForCreateBlocks (decoded)
pub fn hook_9(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = use_own_wrench_logic_for_create_blocks(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler::rightClickToCopy (decoded)
pub fn hook_10(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = right_click_to_copy(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$LeftClickBlock → lambda com/simibubi/create/content/equipment/clipboard/ClipboardValueSettingsHandler::leftClickToPaste (decoded)
pub fn hook_11(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], state: horizon_world::registry::BlockStateId, canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("left_click_block"),
    );
    let _result = left_click_to_paste(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/level/BlockEvent$BreakEvent → lambda com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem::consumeDurabilityOnBlockBreak (decoded)
pub fn hook_12(game: &mut crate::game::Game<'_>, pos: [i32; 3], state: horizon_world::registry::BlockStateId, canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("destroy_block"),
    );
    let _result = consume_durability_on_block_break(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$EntityInteract → lambda com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem::notifyServerOfLongRangeInteractions (decoded)
pub fn hook_13(game: &mut crate::game::Game<'_>, entity: i32, kind: &str, stack: Option<&horizon_client::items::ItemStack>, claimed: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("interact_entity"),
    );
    let _result = notify_server_of_long_range_interactions(&mut ctx, JvmVal::Null);
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$EntityInteractSpecific → lambda com/simibubi/create/content/equipment/extendoGrip/ExtendoGripItem::notifyServerOfLongRangeSpecificInteractions (decoded)
pub fn hook_14(game: &mut crate::game::Game<'_>, entity: i32, kind: &str, stack: Option<&horizon_client::items::ItemStack>, claimed: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("interact_entity"),
    );
    let _result = notify_server_of_long_range_specific_interactions(&mut ctx, JvmVal::Null);
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/redstone/link/LinkHandler::onBlockActivated (decoded)
pub fn hook_15(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = on_block_activated_2(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/redstone/displayLink/ClickToLinkBlockItem::linkableItemAlwaysPlacesWhenUsed (decoded)
pub fn hook_16(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = linkable_item_always_places_when_used(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/logistics/depot/EjectorTargetHandler::rightClickingBlocksSelectsThem (decoded)
pub fn hook_17(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = right_clicking_blocks_selects_them_2(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$LeftClickBlock → lambda com/simibubi/create/content/logistics/depot/EjectorTargetHandler::leftClickingBlocksDeselectsThem (decoded)
pub fn hook_18(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], state: horizon_world::registry::BlockStateId, canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("left_click_block"),
    );
    let _result = left_clicking_blocks_deselects_them_2(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$EntityInteractSpecific → lambda com/simibubi/create/content/logistics/stockTicker/StockTickerInteractionHandler::interactWithLogisticsManager (decoded)
pub fn hook_19(game: &mut crate::game::Game<'_>, entity: i32, kind: &str, stack: Option<&horizon_client::items::ItemStack>, claimed: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("interact_entity"),
    );
    let _result = interact_with_logistics_manager(&mut ctx, JvmVal::Null);
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/logistics/itemHatch/ItemHatchHandler::useOnItemHatchIgnoresSneak (decoded)
pub fn hook_20(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = use_on_item_hatch_ignores_sneak(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/logistics/funnel/FunnelItem::funnelItemAlwaysPlacesWhenUsed (decoded)
pub fn hook_21(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = funnel_item_always_places_when_used(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$EntityInteractSpecific → lambda com/simibubi/create/content/trains/schedule/ScheduleItemEntityInteraction::interactWithConductor (decoded)
pub fn hook_22(game: &mut crate::game::Game<'_>, entity: i32, kind: &str, stack: Option<&horizon_client::items::ItemStack>, claimed: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("interact_entity"),
    );
    let _result = interact_with_conductor(&mut ctx, JvmVal::Null);
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/trains/track/TrackBlockItem::sendExtenderPacket (decoded)
pub fn hook_23(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = send_extender_packet(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickItem → lambda com/simibubi/create/content/fluids/FluidBottleItemHook::preventWaterBottlesFromCreatesFluids (decoded)
pub fn hook_24(game: &mut crate::game::Game<'_>, stack: &horizon_client::items::ItemStack, claimed: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("use_item"),
    );
    let _result = prevent_water_bottles_from_creates_fluids(&mut ctx, JvmVal::Null);
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$EntityInteract → lambda com/simibubi/create/content/contraptions/mounted/MinecartContraptionItem::wrenchCanBeUsedToPickUpMinecartContraptions (decoded)
pub fn hook_25(game: &mut crate::game::Game<'_>, entity: i32, kind: &str, stack: Option<&horizon_client::items::ItemStack>, claimed: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("interact_entity"),
    );
    let _result = wrench_can_be_used_to_pick_up_minecart_contraptions(&mut ctx, JvmVal::Null);
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$EntityInteract → lambda com/simibubi/create/content/contraptions/minecart/MinecartCouplingItem::handleInteractionWithMinecart (decoded)
pub fn hook_26(game: &mut crate::game::Game<'_>, entity: i32, kind: &str, stack: Option<&horizon_client::items::ItemStack>, claimed: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("interact_entity"),
    );
    let _result = handle_interaction_with_minecart(&mut ctx, JvmVal::Null);
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/contraptions/glue/SuperGlueItem::glueItemAlwaysPlacesWhenUsed (decoded)
pub fn hook_27(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = glue_item_always_places_when_used(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/RegisterCommandsEvent → lambda com/simibubi/create/foundation/events/CommonEvents::registerCommands (decoded)
pub fn hook_28(game: &mut crate::game::Game<'_>, commands: &mut Vec<crate::game::native_server::NativeCommand>) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("register_commands"),
    );
    let _result = register_commands(&mut ctx, JvmVal::Null);
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/server/ServerStoppingEvent → lambda com/simibubi/create/foundation/events/CommonEvents::serverStopping (decoded)
pub fn hook_29(game: &mut crate::game::Game<'_>, dedicated: bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("server_stopping"),
    );
    let _result = server_stopping(&mut ctx, JvmVal::Null);
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/foundation/blockEntity/behaviour/edgeInteraction/EdgeInteractionHandler::onBlockActivated (decoded)
pub fn hook_30(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = on_block_activated_3(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/foundation/blockEntity/behaviour/ValueSettingsInputHandler::onBlockActivated (decoded)
pub fn hook_31(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = on_block_activated_4(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/foundation/block/ItemUseOverrides::onBlockActivated (decoded)
pub fn hook_32(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    let mut ctx = BridgeCtx::new(
        GameRef::Game(game),
        ctx_dimension("right_click_block"),
    );
    let _result = on_block_activated_5(&mut ctx, JvmVal::BlockPos(pos));
    drop(_result);
}

// # source: fabric event net/neoforged/neoforge/event/RegisterCommandsEvent → lambda net/createmod/ponder/NeoForgePonder$Events::registerCommands (decoded)
pub fn hook_33(game: &mut crate::game::Game<'_>, commands: &mut Vec<crate::game::native_server::NativeCommand>) {
    let _ = (commands); // event with no decoded callback — safe no-op
}

// === CREATE :: KINETIC CORE — hand-written Rust (no transpiler) ===
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


// === CREATE :: PONDER GUIDE DATA (from Create's en_us.json) ===
// Ponder guide pages, curated from Create's en_us.json (create.ponder.*).
pub static GUIDE: &[(&str, &str, &[&str])] = &[
    ("shaft", "Relaying rotational force using Shafts", &["Shafts will relay rotation in a straight line."]),
    ("cogwheel", "Relaying rotational force using Cogwheels", &["Cogwheels will relay rotation to other adjacent cogwheels", "Neighbouring shafts connected like this will rotate in opposite directions"]),
    ("large_cogwheel", "Relaying rotational force using Large Cogwheels", &["Large cogwheels can connect to each other at right angles", "It will help relaying conveyed speed to other axes of rotation"]),
    ("gearbox", "Relaying rotational force using Gearboxes", &["Jumping between axes of rotation can get bulky quickly", "A gearbox is the more compact equivalent of this setup", "Shafts around corners rotate in mirrored directions", "Straight connections will be reversed"]),
    ("water_wheel", "Generating Rotational Force using Water Wheels", &["Water Wheels draw force from adjacent Water Currents", "Covering additional sides will not improve its kinetic output further", "Use wood planks on the wheel to change its appearance"]),
    ("steam_engine", "Setting up Steam Engines", &["Steam Engines can be placed on a Fluid Tank", "Clicking the engine with a Shaft creates the Kinetic Output", "With sufficient Heat, Water and Boiler space...", "...they will generate Rotational Force", "The minimal setup requires 4 Fluid Tanks", "With the help of Blaze Burners, the power output can be increased", "Higher power levels require more Water, Size and Heat", "The boiler's current power level can be inspected with Engineer's Goggles", "With each added power level, an additional Engine can output at full capacity", "Lvl 4", "4 Engines", "Lvl 8", "8 Engines"]),
    ("millstone", "Processing Items in the Millstone", &["Millstones process items by grinding them", "They can be powered from the side using cogwheels", "Throw or Insert items at the top", "After some time, the result can be obtained via Right-click", "The outputs can also be extracted by automation"]),
    ("mechanical_drill", "Breaking Blocks with the Mechanical Drill", &["When given Rotational Force, a Mechanical Drill will break blocks directly in front of it", "Its mining speed depends on the Rotational Input"]),
    ("deployer", "Using the Deployer", &["Given Rotational Force, a Deployer can imitate player interactions", "It will always interact with the position 2 blocks in front of itself", "Blocks directly in front will not obstruct it", "Deployers can:", "Place Blocks,", "Use Items,", "Activate Blocks,", "Harvest blocks", "and Attack Mobs", "Right-click the front to give it an Item to use", "Items can also be inserted automatically", "Deployers carry a filter slot", "When a filter is set, it activates only while holding a matching item", "Only items matching the filter can now be inserted...", "...and only non-matching items will be extracted"]),
    ("mechanical_press", "Processing Items with the Mechanical Press", &["The Mechanical Press can process items provided beneath it", "The Input items can be dropped or placed on a Depot under the Press", "When items are provided on a belt...", "The Press will hold and process them automatically"]),
];

// ---- hand hook handlers ------------------------------------------------------

/// native_block_entity: attach a kinetic block entity to every Create
/// kinetic block, remembering which block it belongs to.
pub fn hook_block_entity(block: &str, entity: &mut Option<(String, horizon_world::nbt::Compound)>) {
    if kinetic_kind(block).is_some() {
        if entity.is_none() {
            let mut nbt = horizon_world::nbt::Compound::new();
            nbt.insert(
                horizon_world::nbt::JavaString::from("j2h:block"),
                horizon_world::nbt::Tag::String(horizon_world::nbt::JavaString::from(block)),
            );
            *entity = Some(("create:kinetic".into(), nbt));
        }
    }
    // The guide block gets its own block entity (its page index rides the
    // data slots to the client).
    if block == GUIDE_BLOCK && entity.is_none() {
        *entity = Some(("create:guide".into(), horizon_world::nbt::Compound::new()));
    }
}

/// native_block_entity_tick: register the position in the network, advance
/// the rotation angle, and run consumer processing (crushing chain).
pub fn hook_block_entity_tick(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    entity: &mut horizon_client::containers::ModBlockEntity,
) {
    if entity.id != "create:kinetic" {
        return;
    }
    let key = horizon_world::nbt::JavaString::from("j2h:block");
    if let Some(horizon_world::nbt::Tag::String(b)) = entity.nbt.get(&key) {
        let leaked: &'static str = Box::leak(b.to_string_lossy().into_boxed_str());
        net_attach(pos, leaked);
    }
    net_recompute();
    let (rpm, dir, demand, capacity) = net_speed(pos);
    let old = entity.data.first().copied().unwrap_or(0);
    entity.data = vec![old.wrapping_add((rpm * dir as f32) as i32)];

    // Consumers grind the block beneath them while the network spins.
    let name = entity
        .nbt
        .get(&key)
        .and_then(|t| match t {
            horizon_world::nbt::Tag::String(s) => Some(s.to_string_lossy()),
            _ => None,
        })
        .unwrap_or_default();
    if kinetic_kind(&name) == Some(KinKind::Consumer) {
        let mut ctx = BridgeCtx::new(GameRef::Game(game), String::new());
        let mut counter = entity.data.get(1).copied().unwrap_or(0);
        consumer_tick(&mut ctx, pos, &mut counter);
        if entity.data.len() < 2 {
            entity.data.push(counter);
        } else {
            entity.data[1] = counter;
        }
    }
}

/// native_right_click_block: report the network's state at this position.
pub fn hook_right_click(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    _face: [i32; 3],
    _canceled: &mut bool,
) {
    let (rpm, dir, demand, capacity) = net_speed(pos);
    let arrow = match dir {
        d if d < 0 => "\u{21ba}",
        d if d > 0 => "\u{21bb}",
        _ => "\u{00b7}",
    };
    let msg = format!(
        "Create kinetics: {arrow} {:.0} RPM | stress {demand}/{capacity} SU",
        rpm.abs()
    );
    game.native_chat(&msg);
}

/// native_tooltip: Create's summaries for the kinetic items.
pub fn hook_tooltip(
    stack: &horizon_client::items::ItemStack,
    lines: &mut Vec<(horizon_client::text::Text, u32)>,
) {
    if let Some(summary) = tooltip_summary(&stack.item) {
        lines.push((horizon_client::text::Text::literal(summary), 14540253));
    }
}

/// native_screen: the guide block opens the Ponder screen.
pub fn hook_guide_screen(block: &str, screen: &mut Option<crate::game::native_hooks::NativeScreen>) {
    guide_screen_attach(block, screen);
}

/// native_screen_draw data: (title, wrapped lines) of the current page.
pub fn hook_guide_draw(data: &[i32]) -> (String, Vec<String>) {
    let page = guide_page_of(data);
    let (_subject, lines) = guide_page(page);
    (
        format!("Create \u{2014} Ponder {}/{}", page + 1, GUIDE.len()),
        lines.into_iter().map(str::to_string).collect(),
    )
}

/// native_screen_click: page-turn zones -> menu button ids.
pub fn hook_guide_click(at: [f32; 2]) -> Option<i32> {
    let (x, y) = (at[0], at[1]);
    if (8.0..24.0).contains(&x) && (140.0..156.0).contains(&y) {
        return Some(0); // prev page
    }
    if (152.0..168.0).contains(&x) && (140.0..156.0).contains(&y) {
        return Some(1); // next page
    }
    None
}

/// native_menu_button (server): turn the page in the BE's data slots.
pub fn hook_guide_menu_button(entity: &mut horizon_client::containers::ModBlockEntity, id: i32) {
    if entity.id != "create:guide" {
        return;
    }
    let page = entity.data.first().copied().unwrap_or(0);
    let next = match id {
        0 => page - 1,
        1 => page + 1,
        _ => page,
    };
    if entity.data.is_empty() {
        entity.data.push(next);
    } else {
        entity.data[0] = next;
    }
}
