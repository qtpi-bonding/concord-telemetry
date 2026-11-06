fn bad() { match work() { Err(e) => e.to_string(), Ok(_) => String::new() } }
fn good() { match work() { Err(e) => format!("{e:?}"), Ok(_) => String::new() } }
