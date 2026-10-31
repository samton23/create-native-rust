// # source: fabric event net/neoforged/neoforge/event/server/ServerStoppingEvent → lambda com/simibubi/create/foundation/events/CommonEvents::serverStopping (decoded)
// Horizon native hook `server_stopping` (1 params). Returning unit leaves
// the native method body/result intact.
fn hook_29(game: &mut crate::game::Game<'_>, dedicated: bool) {
    crate::game::create::hook_29(game, dedicated);
}
