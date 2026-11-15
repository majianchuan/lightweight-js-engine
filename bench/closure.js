function make(n) {
  return function (x) { return x + n; };
}
const f = make(3);
let s = 0;
for (let i = 0; i < 2000000; i++) s = f(s) & 0xffff;
print(s);
