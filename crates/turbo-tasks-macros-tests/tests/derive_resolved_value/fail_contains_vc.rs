#![allow(dead_code)]

use turbo_tasks::{ResolvedValue, ResolvedVc, Vc};

#[derive(ResolvedValue)]
struct ContainsOnlyVc {
    a: Vc<i32>,
}

#[derive(ResolvedValue)]
struct ContainsResolvedVcAndVc {
    a: ResolvedVc<i32>,
    b: Vc<i32>,
}

#[derive(ResolvedValue)]
struct ContainsVcInsideGeneric {
    a: Option<Box<[Vc<i32>; 4]>>,
}

fn main() {}
