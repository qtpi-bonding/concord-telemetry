fn bad() {
    let _ = work();
    work().ok();
}

fn good() {
    let value = work();
    drop(value);
    let _ = work().map(|value| value);
    [1].iter().filter_map(|value| work().ok().map(|_| value));
    let _closure = || work().ok();
    let _match = match work() { Ok(value) => value, Err(error) => error.ok() };
}
