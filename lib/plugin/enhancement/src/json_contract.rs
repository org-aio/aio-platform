use serde_json::{Number, Value};

// JSON 的 1 与 1.0 表示相同数值。脚本往返可以改变数字写法，但不能舍入大整数。
pub(crate) fn equivalent(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => numbers_equal(left, right),
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| equivalent(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .all(|(key, left)| right.get(key).is_some_and(|right| equivalent(left, right)))
        }
        _ => left == right,
    }
}

fn numbers_equal(left: &Number, right: &Number) -> bool {
    if left == right {
        return true;
    }
    let (float, integer) = match (left.is_f64(), right.is_f64()) {
        (true, false) => (left, right),
        (false, true) => (right, left),
        _ => return false,
    };
    let Some(value) = float.as_f64() else {
        return false;
    };
    if value.fract() != 0.0 {
        return false;
    }
    // 上界必须排除 2^63 / 2^64，避免 Rust 浮点转整数的饱和行为制造相等。
    if let Some(integer) = integer.as_i64() {
        return (i64::MIN as f64..-(i64::MIN as f64)).contains(&value) && value as i64 == integer;
    }
    integer.as_u64().is_some_and(|integer| {
        (0.0..18_446_744_073_709_551_616.0).contains(&value) && value as u64 == integer
    })
}
