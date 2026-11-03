// Compiled into the client library, which can't see `ntm_direct` or `ntm_core`.
fn ntm_menu_slot_rules(menu: &str, slot: usize, stack: &ItemStack, may_place: &mut bool) {
    if menu == "ntm:shredder" {
        let blade = stack.item.starts_with("ntm:blades_");
        *may_place = match slot {
            0..=8 => !blade,
            27 | 28 => blade,
            _ => false,
        };
    }
}
