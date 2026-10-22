// oracle: levels 4
// A numbering function and a wide number column: the numbers right-align to the widest.
#set page(width: 300pt, height: 400pt, margin: 25pt)
#enum(numbering: n => [Step #n:])[first][second][third]
#set enum(number-align: start)
+ a
+ b
#set enum(number-align: end)
#enum(start: 9)[nine][ten][eleven]
