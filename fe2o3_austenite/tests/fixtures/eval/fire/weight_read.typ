// A weight reads back by its name where it has one, as Typst keeps it (abbr 0.3.0's `text.weight <= "medium"`).
#context repr(text.weight) #[#set text(weight: 700); #context repr(text.weight)]
#[#set text(weight: 450); #context repr(text.weight)] #[#set text(weight: 1000); #context repr(text.weight)]
#[#set text(weight: 300); #context if text.weight <= "medium" [light] else [heavy]]
