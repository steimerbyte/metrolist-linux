//! Prints the generated boot registry, one line per module.

use ml_boot::{total, MODULES};

fn main() {
    let total_ms = (total(MODULES.len()) * 1000.0).round() as u64;
    println!("BOOT modules={} total_ms={total_ms}", MODULES.len());
    for module in MODULES {
        let init = if module.init.is_some() {
            "some"
        } else {
            "none"
        };
        println!(
            "BOOT module={} label={} order={} init={init} status=ok",
            module.name, module.label, module.order
        );
    }
}
