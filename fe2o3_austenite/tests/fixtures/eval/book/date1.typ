// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#let d = datetime(year: 2024, month: 3, day: 5)
#d.display("[day] [month repr:long] [year]") #d.year() #d.month() #d.weekday()
#datetime(year: 2020, month: 1, day: 1, hour: 12, minute: 30, second: 0).display("[hour]:[minute]")
#calc.floor(3.7) #calc.round(3.14159, digits: 2) #calc.max(1, 2, 3) #calc.rem(7, 3) #calc.pow(2, 10) #calc.abs(-4) #calc.odd(3) #calc.min(4, 2)
