use oxedyne_fe2o3_geom::shape::{ShapeF32, ShapeF64, ShapeU32};

#[test]
fn test_shapef32_area() {
    // Circle with radius 5.
    let circle = ShapeF32::Circle(5.0);
    assert!((circle.area() - 78.53982).abs() < 0.001);
    
    // Rectangle 4x6.
    let rect = ShapeF32::Rectangle(4.0, 6.0);
    assert_eq!(rect.area(), 24.0);
    
    // Square with side 3.
    let square = ShapeF32::Square(3.0);
    assert_eq!(square.area(), 9.0);
}

#[test]
fn test_shapef32_perimeter() {
    // Circle with radius 5.
    let circle = ShapeF32::Circle(5.0);
    assert!((circle.perimeter() - 31.41593).abs() < 0.001);
    
    // Rectangle 4x6.
    let rect = ShapeF32::Rectangle(4.0, 6.0);
    assert_eq!(rect.perimeter(), 20.0);
    
    // Square with side 3.
    let square = ShapeF32::Square(3.0);
    assert_eq!(square.perimeter(), 12.0);
}

#[test]
fn test_shapef64_area() {
    // Circle with radius 10.
    let circle = ShapeF64::Circle(10.0);
    assert!((circle.area() - 314.159265358979).abs() < 0.00001);
    
    // Rectangle 5x8.
    let rect = ShapeF64::Rectangle(5.0, 8.0);
    assert_eq!(rect.area(), 40.0);
    
    // Square with side 7.
    let square = ShapeF64::Square(7.0);
    assert_eq!(square.area(), 49.0);
}

#[test]
fn test_shapef64_perimeter() {
    // Circle with radius 10.
    let circle = ShapeF64::Circle(10.0);
    assert!((circle.perimeter() - 62.83185307179586).abs() < 0.00001);
    
    // Rectangle 5x8.
    let rect = ShapeF64::Rectangle(5.0, 8.0);
    assert_eq!(rect.perimeter(), 26.0);
    
    // Square with side 7.
    let square = ShapeF64::Square(7.0);
    assert_eq!(square.perimeter(), 28.0);
}

#[test]
fn test_shapeu32_area() {
    // Circle with radius 10.
    let circle = ShapeU32::Circle(10);
    // PI ≈ 355/113, so area ≈ (355 * 100) / 113 = 314.
    assert_eq!(circle.area(), 314);
    
    // Rectangle 5x8.
    let rect = ShapeU32::Rectangle(5, 8);
    assert_eq!(rect.area(), 40);
    
    // Square with side 7.
    let square = ShapeU32::Square(7);
    assert_eq!(square.area(), 49);
}

#[test]
fn test_shapeu32_perimeter() {
    // Circle with radius 10.
    let circle = ShapeU32::Circle(10);
    // 2*PI ≈ 2*355/113, so perimeter ≈ (2 * 355 * 10) / 113 = 62.
    assert_eq!(circle.perimeter(), 62);
    
    // Rectangle 5x8.
    let rect = ShapeU32::Rectangle(5, 8);
    assert_eq!(rect.perimeter(), 26);
    
    // Square with side 7.
    let square = ShapeU32::Square(7);
    assert_eq!(square.perimeter(), 28);
}

#[test]
fn test_shapef32_scale_by_area() {
    // Circle with radius 5, scale by 4.
    let circle = ShapeF32::Circle(5.0);
    let scaled = circle.scale_by_area(4.0);
    assert!((scaled.area() - circle.area() * 4.0).abs() < 0.001);
    
    // Rectangle 3x4, scale by 2.25.
    let rect = ShapeF32::Rectangle(3.0, 4.0);
    let scaled = rect.scale_by_area(2.25);
    assert!((scaled.area() - rect.area() * 2.25).abs() < 0.001);
    // Check aspect ratio preserved.
    if let ShapeF32::Rectangle(w, h) = scaled {
        assert!((w / h - 3.0 / 4.0).abs() < 0.001);
    }
    
    // Square with side 2, scale by 9.
    let square = ShapeF32::Square(2.0);
    let scaled = square.scale_by_area(9.0);
    assert!((scaled.area() - square.area() * 9.0).abs() < 0.001);
}

#[test]
fn test_shapef64_scale_by_area() {
    // Circle with radius 10, scale by 0.25.
    let circle = ShapeF64::Circle(10.0);
    let scaled = circle.scale_by_area(0.25);
    assert!((scaled.area() - circle.area() * 0.25).abs() < 0.00001);
    
    // Rectangle 5x8, scale by 1.44.
    let rect = ShapeF64::Rectangle(5.0, 8.0);
    let scaled = rect.scale_by_area(1.44);
    assert!((scaled.area() - rect.area() * 1.44).abs() < 0.00001);
    // Check aspect ratio preserved.
    if let ShapeF64::Rectangle(w, h) = scaled {
        assert!((w / h - 5.0 / 8.0).abs() < 0.00001);
    }
    
    // Square with side 7, scale by 0.5.
    let square = ShapeF64::Square(7.0);
    let scaled = square.scale_by_area(0.5);
    assert!((scaled.area() - square.area() * 0.5).abs() < 0.00001);
}

#[test]
fn test_shapeu32_scale_by_area() {
    // Circle with radius 10, scale by 4.
    let circle = ShapeU32::Circle(10);
    let scaled = circle.scale_by_area(4);
    // Allow for integer rounding.
    let expected_area = circle.area() * 4;
    let actual_area = scaled.area();
    assert!((actual_area as i32 - expected_area as i32).abs() < 10);
    
    // Rectangle 6x8, scale by 4.
    let rect = ShapeU32::Rectangle(6, 8);
    let scaled = rect.scale_by_area(4);
    assert_eq!(scaled.area(), rect.area() * 4);
    // Check aspect ratio preserved.
    if let ShapeU32::Rectangle(w, h) = scaled {
        assert_eq!(w * 8, h * 6); // Cross multiply to check ratio.
    }
    
    // Square with side 5, scale by 9.
    let square = ShapeU32::Square(5);
    let scaled = square.scale_by_area(9);
    assert_eq!(scaled.area(), square.area() * 9);
}