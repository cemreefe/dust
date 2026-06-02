use std::f64::consts::PI;

enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Triangle(f64, f64),
}

fn area(shape: &Shape) -> f64 {
    match shape {
        Shape::Circle(radius)         => PI * radius * radius,
        Shape::Rect(width, height)    => width * height,
        Shape::Triangle(base, height) => 0.5 * base * height,
    }
}

fn describe(shape: &Shape) -> String {
    match shape {
        Shape::Circle(radius)         => format!("circle with radius {}", radius),
        Shape::Rect(width, height)    => format!("rectangle {}x{}", width, height),
        Shape::Triangle(base, height) => format!("triangle base={} height={}", base, height),
    }
}

fn main() {
    let shapes = vec![Shape::Circle(3.0), Shape::Rect(4.0, 5.0), Shape::Triangle(6.0, 8.0)];
    for shape in &shapes {
        let area = area(shape);
        println!("{}: area = {:.2}", describe(shape), area);
    }
}
