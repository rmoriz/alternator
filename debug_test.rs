fn main() {
    let input = format!("Text{}with{}null{}bytes", '\0', '\x01', '\x02');
    println!("Input: '{}'", input);
    println!("Input length: {}", input.len());
    for (i, c) in input.chars().enumerate() {
        println!("  char {}: '{}' ({})", i, c, c as u32);
    }
    
    let cleaned: String = input
        .chars()
        .filter(|&c| {
            c == '\n' || c == '\t' || (!c.is_control() && c != '\0')
        })
        .collect();
    println!("Cleaned: '{}'", cleaned);
    println!("Cleaned length: {}", cleaned.len());
}
