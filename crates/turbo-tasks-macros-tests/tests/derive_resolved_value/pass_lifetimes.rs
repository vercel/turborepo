use turbo_tasks::ResolvedValue;

#[derive(ResolvedValue)]
struct ContainsBorrowedData<'a> {
    borrowed: &'a str,
}

fn main() {
    let a = ContainsBorrowedData { borrowed: "value" };
    let _ = a.borrowed;
}
