use log::{debug, info};

fn do_cool_stuff() {
    info!("Hello, world!");
    debug!("Hello, world, in detail!");
}

fn main() {
    log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Debug);

    do_cool_stuff();
}
