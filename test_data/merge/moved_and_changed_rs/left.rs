use std::collections::HashMap;


pub struct Inventory {
    items: HashMap<String, u32>,
}


impl Inventory {
    pub fn new() -> Self {
        Inventory { items: HashMap::new() }
    }


    pub fn add(&mut self, name: &str, count: u32) {
        *self.items.entry(name.to_string()).or_insert(0) += count;
    }


    pub fn remove(&mut self, name: &str, count: u32) -> bool {
        match self.items.get_mut(name) {
            Some(stock) if *stock >= count => {
                *stock -= count;
                true
            }
            _ => false,
        }
    }


    pub fn total(&self) -> u32 {
        self.items.values().sum()
    }


    pub fn count(&self, name: &str) -> u32 {
        self.items.get(name).copied().unwrap_or(0)
    }
}


pub fn load(text: &str) -> Inventory {
    let mut inventory = Inventory::new();
    for line in text.lines() {
        if let Some((name, count)) = parse_line(line) {
            inventory.add(&name, count);
        }
    }
    inventory
}


pub fn report(inventory: &Inventory, names: &[&str]) -> String {
    let mut out = String::new();
    for name in names {
        out.push_str(&format!("{name}: {}\n", inventory.count(name)));
    }
    out.push_str(&format!("total: {}\n", inventory.total()));
    out
}


pub fn parse_line(line: &str) -> Option<(String, u32)> {
    let (name, count) = line.split_once(':')?;
    let count = count.trim().parse().ok()?;
    Some((name.trim().to_string(), count))
}
