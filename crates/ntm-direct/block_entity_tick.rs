fn ntm_block_entity_tick(game: &mut Game<'_>, pos: [i32; 3], entity: &mut ModBlockEntity) {
    super::ntm_direct::tick(game, pos, entity);
}
