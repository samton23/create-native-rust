// # source: fabric event net/neoforged/neoforge/event/RegisterCommandsEvent → lambda net/createmod/ponder/NeoForgePonder$Events::registerCommands (decoded)
// Horizon native hook `register_commands` (1 params). Returning unit leaves
// the native method body/result intact.
fn hook_33(game: &mut crate::game::Game<'_>, commands: &mut Vec<crate::game::native_server::NativeCommand>) {
    crate::game::create::hook_33(game, commands);
}
