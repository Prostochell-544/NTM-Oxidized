fn ntm_level_tick_pre(game: &mut Game<'_>, dimension: &str) {
    super::ntm_direct::level_tick(game, dimension);
}
