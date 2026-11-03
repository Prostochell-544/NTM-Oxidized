use horizon_client::mod_api::{Hooks, Host, ServerTickEvent};
use std::sync::Once;

struct NtmHooks;

impl Hooks for NtmHooks {
    fn server_tick(&self, _host: &mut dyn Host, _event: &mut ServerTickEvent<'_>) {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            // The game's log keeps info lines only from targets starting with "horizon".
            log::info!(
                target: "horizon_ntm_api",
                "[ntm_api] {} ticking on thread {:?}",
                ntm_core::NAME,
                std::thread::current().name()
            );
        });
    }
}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(NtmHooks)
}
