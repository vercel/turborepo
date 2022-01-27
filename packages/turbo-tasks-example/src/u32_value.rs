#[turbo_tasks::value]
pub struct I32Value {
    pub value: u32,
}

#[turbo_tasks::value_impl]
impl I32Value {
    #[turbo_tasks::constructor]
    pub fn new(value: u32) -> Self {
        Self { value }
    }
}
