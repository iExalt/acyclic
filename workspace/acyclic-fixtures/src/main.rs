//! `acyclic-fixtures --items <n> --seed <s>`: prints a synthetic plan in the import format.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut items = acyclic_fixtures::BASE_ITEMS;
    let mut seed = 1;
    for pair in args.chunks(2) {
        let parsed = match pair {
            [flag, value] if flag == "--items" => value.parse().map(|n| items = n).is_ok(),
            [flag, value] if flag == "--seed" => value.parse().map(|n| seed = n).is_ok(),
            _ => false,
        };
        if !parsed || items < 50 {
            eprintln!("usage: acyclic-fixtures [--items <n, at least 50>] [--seed <u64>]");
            return ExitCode::from(2);
        }
    }
    println!("{}", acyclic_fixtures::generate(items, seed).to_json());
    ExitCode::SUCCESS
}
