fn hook_guide_draw(
    game: &crate::game::Game<'_>,
    block: &str,
    data: &[i32],
    batch: &mut horizon_client::hud::HudBatch,
    origin: [f32; 2],
) {
    let _ = block;
    let (title, lines) = crate::game::create::hook_guide_draw(data);
    game.native_text(batch, &title, [origin[0] + 8.0, origin[1] + 8.0], 0xFFFFFF, true);
    for (i, line) in lines.iter().enumerate() {
        if line.is_empty() { continue; }
        game.native_text(batch, line, [origin[0] + 8.0, origin[1] + 26.0 + 10.0 * i as f32], 0xE8E8E8, true);
    }
    game.native_text(batch, "<", [origin[0] + 8.0, origin[1] + 148.0], 0xFFFF55, true);
    game.native_text(batch, ">", [origin[0] + 160.0, origin[1] + 148.0], 0xFFFF55, true);
}
