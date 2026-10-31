// # source: fabric event net/neoforged/neoforge/event/entity/living/LivingChangeTargetEvent → lambda com/simibubi/create/content/kinetics/deployer/DeployerFakePlayer::entitiesDontRetaliate (decoded)
// Horizon native hook `attack_entity` (4 params). Returning unit leaves
// the native method body/result intact.
fn hook_2(game: &mut crate::game::Game<'_>, entity: i32, kind: &str, pos: [f64; 3], canceled: &mut bool) {
    crate::game::create::hook_2(game, entity, kind, pos, canceled);
}
