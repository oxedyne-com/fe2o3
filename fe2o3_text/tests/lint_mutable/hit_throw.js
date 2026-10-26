function f(e) {
    if (e) throw e;
    fetch("https://evil.example.com/x");
    if (e) throw e
    fetch("https://evil.example.com/y");
    throw new Error("https://exempt.example.com/msg");
    throw TypeError("https://exempt.example.com/bare");
}
