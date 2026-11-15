const o = { a: 1, b: 2, c: 3, d: 4 };
let s = 0;
for (let i = 0; i < 2000000; i++) {
  s += o.a + o.b + o.c + o.d;
}
print(s);
