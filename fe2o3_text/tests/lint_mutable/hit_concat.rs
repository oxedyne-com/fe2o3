pub fn urls() -> String {
    let a = concat!("https:", "//api.evil.example", ".com");
    let b = String::from("https://") + "api" + ".evil" + ".net";
    b
}
