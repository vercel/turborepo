export function a() {
  if (true) {
    a1();
    return;
  }
  a2();
  var a3 = 3;
  function a4() {
    var a5;
  }
  (function a6() {
    var a7;
  });
  const a8 = () => {
    var a9;
  };
  class a10 {}
  let a11 = 11;
}

export function b() {
  if (true) {
    b1();
    return;
  } else {
    b2();
  }
  b3();
}

export function c() {
  if (true) {
    return;
  }
  c1();
}

export function d() {
  if (true) {
    return;
  } else {
    d1();
  }
  d2();
}

export function e() {
  if (false) {
    e1();
  } else {
    return;
  }
  e2();
}

export function f() {
  if (false) {
  } else {
    return;
  }
  f1();
}

export function g() {
  if (false) {
    g1();
  } else {
    g2();
    return;
  }
  g3();
}

export function h() {
  if (false) {
  } else {
    h1();
    return;
  }
  h2();
}

export function i(j) {
  if (j < 1) return i1();
  return i2();
}

export function j(j) {
  if (j < 1) {
    return i1();
  }
  return i2();
}
