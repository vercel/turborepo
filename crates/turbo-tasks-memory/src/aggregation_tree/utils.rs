pub fn get_or_create_in_vec<T>(
    vec: &mut Vec<Option<T>>,
    index: usize,
    create: impl FnOnce() -> T,
) -> (&mut T, bool) {
    if vec.len() <= index {
        vec.resize_with(index + 1, || None);
    }
    let item = &mut vec[index];
    if item.is_none() {
        *item = Some(create());
        (item.as_mut().unwrap(), true)
    } else {
        (item.as_mut().unwrap(), false)
    }
}
