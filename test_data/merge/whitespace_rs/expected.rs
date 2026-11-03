pub fn clamp(value: i32, low: i32, high: i32) -> i32 {
	if value < low {
		low
	} else if value > high {
		high
	} else {
		value
	}
}


pub fn average(values: &[i32]) -> Option<i32> {
    if values.is_empty() {
        return Option::None;
    }
    let sum: i32 = values.iter().sum();   
    Some(sum / values.len() as i32)
}


pub fn describe(value: i32) -> &'static str {
    match value {
        0 => "zero",
        1..=9  =>  "small",
        _ => "large",
    }
}
