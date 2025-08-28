#[derive(Clone, Copy, Debug)]
pub enum ShapeF32 {
    Circle(f32),
    Rectangle(f32, f32),
    Square(f32),
}

#[derive(Clone, Copy, Debug)]
pub enum ShapeF64 {
    Circle(f64),
    Rectangle(f64, f64),
    Square(f64),
}

#[derive(Clone, Copy, Debug)]
pub enum ShapeU32 {
    Circle(u32),
    Rectangle(u32, u32),
    Square(u32),
}
