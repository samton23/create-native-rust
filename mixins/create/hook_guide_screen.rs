fn hook_guide_screen(
    game: &crate::game::Game<'_>,
    block: &str,
    screen: &mut Option<crate::game::native_hooks::NativeScreen>,
) {
    let _ = game;
    crate::game::create::hook_guide_screen(block, screen);
}
