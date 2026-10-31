// # source: fabric event net/neoforged/neoforge/event/entity/player/PlayerInteractEvent$EntityInteractSpecific → lambda com/simibubi/create/content/logistics/stockTicker/StockTickerInteractionHandler::interactWithLogisticsManager (decoded)
// Horizon native hook `interact_entity` (4 params). Returning unit leaves
// the native method body/result intact.
fn hook_19(game: &mut crate::game::Game<'_>, entity: i32, kind: &str, stack: Option<&horizon_client::items::ItemStack>, claimed: &mut bool) {
    crate::game::create::hook_19(game, entity, kind, stack, claimed);
}
