// # source: fabric event net/neoforged/neoforge/event/RegisterCommandsEvent → lambda com/simibubi/create/foundation/events/CommonEvents::registerCommands (decoded)
// Horizon native hook `register_commands` (1 params). Returning unit leaves
// the native method body/result intact.
fn hook_28(game: &mut crate::game::Game<'_>, commands: &mut Vec<crate::game::native_server::NativeCommand>) {
    crate::game::create::hook_28(game, commands);
}
