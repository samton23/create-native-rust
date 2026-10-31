// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickBlock → lambda com/simibubi/create/content/logistics/itemHatch/ItemHatchHandler::useOnItemHatchIgnoresSneak (decoded)
// Horizon native hook `right_click_block` (3 params). Returning unit leaves
// the native method body/result intact.
fn hook_20(game: &mut crate::game::Game<'_>, pos: [i32; 3], face: [i32; 3], canceled: &mut bool) {
    crate::game::create::hook_20(game, pos, face, canceled);
}
