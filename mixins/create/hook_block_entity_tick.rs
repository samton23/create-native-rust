fn hook_block_entity_tick(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    entity: &mut horizon_client::containers::ModBlockEntity,
) {
    let _ = game;
    crate::game::create::hook_block_entity_tick(pos, entity);
}
