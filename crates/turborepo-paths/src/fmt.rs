/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root
 * directory of this source tree.
 */

//! Format related utilities of the core path types

use std::{
    fmt,
    fmt::{Display, Formatter},
};

/// formats the path as a quoted string
pub(crate) fn quoted_display<D>(d: &D, f: &mut Formatter) -> fmt::Result
where
    D: Display + ?Sized,
{
    write!(f, "\"{:}\"", d)
}
