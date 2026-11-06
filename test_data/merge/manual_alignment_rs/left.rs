fn first() -> u32 {
    1
}

fn parse(input: &str) -> Option<u32> {
    input.trim().parse().ok()
}

fn last() -> u32 {
    9
}
