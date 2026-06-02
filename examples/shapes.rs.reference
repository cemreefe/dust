use std::f64::consts::PI;

enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Triangle(f64, f64),
}

fn area(shape: &Shape) -> f64 {
    match shape {
        Shape::Circle(r)      => PI * r * r,
        Shape::Rect(w, h)     => w * h,
        Shape::Triangle(b, h) => 0.5 * b * h,
    }
}

fn describe(shape: &Shape) -> String {
    match shape {
        Shape::Circle(r)      => format!("circle with radius {}", r),
        Shape::Rect(w, h)     => format!("rectangle {}x{}", w, h),
        Shape::Triangle(b, h) => format!("triangle base={} height={}", b, h),
    }
}

fn main() {
    let shapes = vec![Shape::Circle(3.0), Shape::Rect(4.0, 5.0), Shape::Triangle(6.0, 8.0)];
    for shape in &shapes {
        let a = area(shape);
        println!("{}: area = {:.2}", describe(shape), a);
    }
}
