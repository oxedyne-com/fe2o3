// oracle: levels 3
// A string given to `eval` resolves its paths from the file that calls `eval`.
#eval("read(\"_data.txt\")") #eval("import \"_lib.typ\": a, c; a + c.len()")
