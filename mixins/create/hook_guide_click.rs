fn hook_guide_click(
    game: &crate::game::Game<'_>,
    block: &str,
    data: &[i32],
    at: [f32; 2],
    right: bool,
    button: &mut Option<i32>,
) {
    let _ = (game, block, data, right);
    *button = crate::game::create::hook_guide_click(at);
}
