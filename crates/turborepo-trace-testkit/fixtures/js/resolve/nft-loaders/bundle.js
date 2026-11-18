!function (e) { module.exports = e() }(function () {
  return function r(l, u, o) {
    function s(t, e) {
      if (!u[t]) {
        if (!l[t]) {
          var i = "function" == typeof require && require;
          if (!e && i) return i(t, !0);
          var n = new Error("Cannot find module '" + t + "'");
          throw n.code = "MODULE_NOT_FOUND", n
        }
        var a = u[t] = { exports: {} };
        l[t][0].call(a.exports, function (e) { return s(l[t][1][e] || e) }, a, a.exports, r, l, u, o)
      }
      return u[t].exports
    }
    for (var c = 0; c < o.length; c++) s(o[c]);
    return s
  }({
    1: [function (s, e, t) {
      var ext = s("ext-dep");
      var local = s("./internal");
    }, { "ext-dep": void 0, "./internal": 2 }],
    2: [function (s, e, t) {
      e.exports = 1;
    }, {}]
  }, {}, [1])(1)
});
