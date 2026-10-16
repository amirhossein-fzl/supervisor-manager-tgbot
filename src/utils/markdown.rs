pub fn replace_special_chars(text: &String) -> String {
    let special_chars = vec![
        '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ];

    text.chars()
        .map(|c| {
            if special_chars.contains(&c) {
                format!("\\{}", c)
            } else {
                c.to_string()
            }
        })
        .collect()
}
