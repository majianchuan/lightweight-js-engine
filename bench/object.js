let last;
for (let i = 0; i < 200000; i++) {
  last = { x: i, y: i + 1, m() { return this.x + this.y; } };
}
print(last.m());
