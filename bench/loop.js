let sum = 0;
for (let i = 0; i < 3000000; i++) {
  sum = (sum + i) | 0;
}
print(sum);
