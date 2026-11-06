fn first() -> u32 {
    1
}

fn parse(input: &str) -> Option<u32> {
    input.trim().parse().ok()
}

fn render(value: u32) -> String {
    format!("<{value}>")
}

fn last() -> u32 {
    9
}
