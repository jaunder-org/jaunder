use std::sync::LazyLock;

pub fn ensure_server_fns_registered() {
    static REGISTRATIONS: LazyLock<()> = LazyLock::new(web::server_fn_registration::register_all);
    LazyLock::force(&REGISTRATIONS);
}
