fn ntm_server_tick() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        log::info!(
            "[ntm_direct] {} ticking on thread {:?}",
            ntm_core::NAME,
            std::thread::current().name()
        );
    });
}
