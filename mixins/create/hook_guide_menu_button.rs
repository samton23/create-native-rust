fn hook_guide_menu_button(
    pos: [i32; 3],
    entity: &mut horizon_client::containers::ModBlockEntity,
    id: i32,
) {
    let _ = pos;
    crate::game::create::hook_guide_menu_button(entity, id);
}
