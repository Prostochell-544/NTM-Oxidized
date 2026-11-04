// Compiled into the client library, which can't see `ntm_direct` or `ntm_core`: the rules of
// `ntm_core::machines::may_place`, written out again.
fn ntm_menu_slot_rules(menu: &str, slot: usize, stack: &ItemStack, may_place: &mut bool) {
    let blade = stack.item.starts_with("ntm:blades_");
    let pack = stack.item.starts_with("ntm:battery_pack_");
    let stamp = stack.item.starts_with("ntm:stamp_");
    let plain = !blade && !pack && !stamp;
    *may_place = match menu {
        "ntm:shredder" => match slot {
            0..=8 => plain,
            27 | 28 => blade,
            29 => pack,
            _ => false,
        },
        "ntm:blast_furnace" => slot <= 2 && plain,
        "ntm:press" => match slot {
            0 | 2 => plain,
            1 => stamp,
            3 => false,
            _ => true,
        },
        "ntm:battery_socket" => pack,
        _ => return,
    };
}
