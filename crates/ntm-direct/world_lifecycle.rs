fn ntm_world_lifecycle(phase: WorldPhase) {
    if matches!(phase, WorldPhase::WorldCloseFinal | WorldPhase::WorldCloseFailed) {
        super::ntm_direct::world_closed();
    }
}
