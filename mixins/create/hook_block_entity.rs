fn hook_block_entity(
    game: &mut crate::game::Game<'_>,
    block: &str,
    entity: &mut Option<(String, horizon_world::nbt::Compound)>,
) {
    let _ = game;
    crate::game::create::hook_block_entity(block, entity);
}
