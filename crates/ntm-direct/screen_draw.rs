fn ntm_screen_draw(
    game: &Game<'_>,
    block: &str,
    data: &[i32],
    batch: &mut HudBatch,
    origin: [f32; 2],
) {
    super::ntm_direct::screen_draw(game, block, data, batch, origin);
}
