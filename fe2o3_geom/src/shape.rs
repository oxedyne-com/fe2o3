use std::f32::consts::PI as PI32;
use std::f64::consts::PI as PI64;

/// A shape with f32 dimensions supporting area and perimeter calculations.
#[derive(Clone, Copy, Debug)]
pub enum ShapeF32 {
    Circle(f32),
    Rectangle(f32, f32),
    Square(f32),
}

impl ShapeF32 {
    /// Calculates the area of the shape.
    /// 
    /// # Returns
    /// The area as an f32 value.
    pub fn area(&self) -> f32 {
        match self {
            Self::Circle(r)		    => PI32 * r * r,
            Self::Rectangle(w, h)	=> w * h,
            Self::Square(s)		    => s * s,
        }
    }
    
    /// Calculates the perimeter of the shape.
    /// 
    /// # Returns
    /// The perimeter as an f32 value.
    pub fn perimeter(&self) -> f32 {
        match self {
            Self::Circle(r)		    => 2.0 * PI32 * r,
            Self::Rectangle(w, h)	=> 2.0 * (w + h),
            Self::Square(s)		    => 4.0 * s,
        }
    }
    
    /// Scales the shape by an area factor while preserving aspect ratio.
    /// 
    /// # Arguments
    /// * `factor` - The area scaling factor (e.g., 2.0 doubles the area).
    /// 
    /// # Returns
    /// A new shape with scaled dimensions.
    pub fn scale_by_area(&self, factor: f32) -> Self {
        let scale = factor.sqrt(); // Linear scale factor.
        match self {
            Self::Circle(r)		    => Self::Circle(r * scale),
            Self::Rectangle(w, h)	=> Self::Rectangle(w * scale, h * scale),
            Self::Square(s)		    => Self::Square(s * scale),
        }
    }
}

/// A shape with f64 dimensions supporting area and perimeter calculations.
#[derive(Clone, Copy, Debug)]
pub enum ShapeF64 {
    Circle(f64),
    Rectangle(f64, f64),
    Square(f64),
}

impl ShapeF64 {
    /// Calculates the area of the shape.
    /// 
    /// # Returns
    /// The area as an f64 value.
    pub fn area(&self) -> f64 {
        match self {
            Self::Circle(r)		    => PI64 * r * r,
            Self::Rectangle(w, h)	=> w * h,
            Self::Square(s)		    => s * s,
        }
    }
    
    /// Calculates the perimeter of the shape.
    /// 
    /// # Returns
    /// The perimeter as an f64 value.
    pub fn perimeter(&self) -> f64 {
        match self {
            Self::Circle(r)		    => 2.0 * PI64 * r,
            Self::Rectangle(w, h)	=> 2.0 * (w + h),
            Self::Square(s)		    => 4.0 * s,
        }
    }
    
    /// Scales the shape by an area factor while preserving aspect ratio.
    /// 
    /// # Arguments
    /// * `factor` - The area scaling factor (e.g., 2.0 doubles the area).
    /// 
    /// # Returns
    /// A new shape with scaled dimensions.
    pub fn scale_by_area(&self, factor: f64) -> Self {
        let scale = factor.sqrt(); // Linear scale factor.
        match self {
            Self::Circle(r)		    => Self::Circle(r * scale),
            Self::Rectangle(w, h)	=> Self::Rectangle(w * scale, h * scale),
            Self::Square(s)		    => Self::Square(s * scale),
        }
    }
}

/// A shape with u32 dimensions supporting area and perimeter calculations.
/// 
/// Note: Circle calculations use integer approximations.
#[derive(Clone, Copy, Debug)]
pub enum ShapeU32 {
    Circle(u32),
    Rectangle(u32, u32),
    Square(u32),
}

impl ShapeU32 {
    /// Calculates the area of the shape.
    /// 
    /// For circles, uses integer approximation with PI ≈ 355/113.
    /// 
    /// # Returns
    /// The area as a u32 value.
    pub fn area(&self) -> u32 {
        match self {
            Self::Circle(r)		=> {
                // Using PI approximation 355/113 for integer arithmetic.
                let r_u64 = *r as u64;
                ((355 * r_u64 * r_u64) / 113) as u32
            },
            Self::Rectangle(w, h)	=> w * h,
            Self::Square(s)		    => s * s,
        }
    }
    
    /// Calculates the perimeter of the shape.
    /// 
    /// For circles, uses integer approximation with PI ≈ 355/113.
    /// 
    /// # Returns
    /// The perimeter as a u32 value.
    pub fn perimeter(&self) -> u32 {
        match self {
            Self::Circle(r)		=> {
                // Using PI approximation 355/113 for integer arithmetic.
                let r_u64 = *r as u64;
                ((2 * 355 * r_u64) / 113) as u32
            },
            Self::Rectangle(w, h)	=> 2 * (w + h),
            Self::Square(s)		    => 4 * s,
        }
    }
    
    /// Scales the shape by an area factor while preserving aspect ratio.
    /// 
    /// Uses integer square root approximation for scaling.
    /// 
    /// # Arguments
    /// * `factor` - The area scaling factor (e.g., 2 doubles the area).
    /// 
    /// # Returns
    /// A new shape with scaled dimensions.
    pub fn scale_by_area(&self, factor: u32) -> Self {
        // Integer square root approximation.
        let scale = (factor as f64).sqrt();
        match self {
            Self::Circle(r)		    => Self::Circle(((*r as f64) * scale) as u32),
            Self::Rectangle(w, h)	=> Self::Rectangle(((*w as f64) * scale) as u32, ((*h as f64) * scale) as u32),
            Self::Square(s)		    => Self::Square(((*s as f64) * scale) as u32),
        }
    }
}
