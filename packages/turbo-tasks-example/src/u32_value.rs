#[turbo_tasks::value]
pub struct U32Value {
    pub value: u32,
}

#[turbo_tasks::value_impl]
impl U32Value {
    #[turbo_tasks::constructor(!interning)]
    pub fn new(value: u32) -> Self {
        Self { value }
    }
}
