// j2h: the mod's entry in Options > Mods (official_minimap pattern).
fn hook_mods(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    mods.push(crate::game::native_hooks::NativeMod {
        id: "create".into(),
        title: format!("Create {} (native j2h)", "6.0.10"),
        description: "Java Create 6.0.10, transpiled to native Rust by j2h".into(),
        keys: Vec::new(),
    });
}
