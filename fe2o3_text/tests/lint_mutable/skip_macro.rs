pub fn f(p: &str) -> Outcome<()> {
    info!("Reading /var/lib/app/state from https://example.com");
    let s = fmt!("{}", "x");
    let t = include_str!("../data/table.json");
    Err(err!("Cannot open './config.jdat' at {}", p; IO, File))
}
