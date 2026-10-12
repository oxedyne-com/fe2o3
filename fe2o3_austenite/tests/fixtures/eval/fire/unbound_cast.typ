// A method reached through its type takes the receiver as a parameter, cast to the type: an integer is
// a float there (lilaq's `float.is-nan(x)` over integers), while `(1).is-nan()` stays an error.
#float.is-nan(1) #float.is-infinite(2) #float.is-nan(float.nan)
#(1, 2.5, float.nan).filter(x => not float.is-nan(x))
