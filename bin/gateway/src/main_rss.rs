use simd_json::{base::ValueAsObject, serde::to_borrowed_value, BorrowedValue};
use sonic_rs::{from_str, JsonContainerTrait, Value};

mod sample_json;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let _profiler = dhat::Profiler::new_heap();

    test_simd();
}

fn test_sonic() {
    let val: Value = from_str(&sample_json::get_sample_json()).unwrap();
    let data = val.as_object().unwrap().get(&"data");

    println!("{}", data.is_some());

    ::std::mem::size_of_val(&val);
}

fn test_simd() {
    let val: BorrowedValue = to_borrowed_value(&mut sample_json::get_sample_json()).unwrap();
    let data = val.as_object().unwrap().get("data");

    println!("{}", data.is_some());

    ::std::mem::size_of_val(&val);
}
