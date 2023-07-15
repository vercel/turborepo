use turbo_tasks::Vc;

pub fn unit() -> Vc<()> {
    Vc::cell(())
}
