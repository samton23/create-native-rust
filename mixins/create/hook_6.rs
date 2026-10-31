// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$LeftClickBlock → lambda com/simibubi/create/content/equipment/tool/CardboardSwordItem::cardboardSwordsMakeNoiseOnClick (decoded)
// Horizon native hook `left_click_block` (4 params). Returning unit leaves
// the native method body/result intact.
fn hook_6(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], state: horizon_world::registry::BlockStateId, canceled: &mut bool) {
    crate::game::create::hook_6(game, pos, face, state, canceled);
}
