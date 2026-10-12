// Code given to `eval` belongs to the file that calls `eval`, as Typst gives its nodes the call's span, so a
// path in it resolves there (linguify's `eval(load_ftl_data(..))` reading its `.ftl` files).
#import "pathlib/evlib.typ": load
#eval("read(\"pathlib/d.json\").len()") #load("d.json")
#eval("[#read(\"pathlib/d.json\")]", mode: "code")
