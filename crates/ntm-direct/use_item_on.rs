fn ntm_use_item_on(game: &mut Game<'_>, stack: &ItemStack, pos: [i32; 3], claimed: &mut bool) {
    super::ntm_direct::use_item_on(game, stack, pos, claimed);
}
