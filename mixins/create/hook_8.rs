// # source: fabric event net/neoforged/neoforge/event/level/BlockEvent$BreakEvent → lambda com/simibubi/create/content/equipment/symmetryWand/SymmetryHandler::onBlockDestroyed (decoded)
// Horizon native hook `destroy_block` (3 params). Returning unit leaves
// the native method body/result intact.
fn hook_8(game: &mut crate::game::Game<'_>, pos: [i32; 3], state: horizon_world::registry::BlockStateId, canceled: &mut bool) {
    crate::game::create::hook_8(game, pos, state, canceled);
}
