fn hook_tooltip(
    game: &mut crate::game::Game<'_>,
    stack: &horizon_client::items::ItemStack,
    lines: &mut Vec<(horizon_client::text::Text, u32)>,
) {
    let _ = game;
    crate::game::create::hook_tooltip(stack, lines);
}
