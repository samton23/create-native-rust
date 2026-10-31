// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$RightClickItem → lambda com/simibubi/create/content/fluids/FluidBottleItemHook::preventWaterBottlesFromCreatesFluids (decoded)
// Horizon native hook `use_item` (2 params). Returning unit leaves
// the native method body/result intact.
fn hook_24(game: &mut crate::game::Game<'_>, stack: &horizon_client::items::ItemStack, claimed: &mut bool) {
    crate::game::create::hook_24(game, stack, claimed);
}
