#!/bin/sh
# What the first four demo panes show, by `AKYBA_PANE_ID`: tests running, a question waiting for
# you, finished work, a migration. Made up, for `pnpm demo` and the screenshots.
g='\033[32m'; r='\033[31m'; y='\033[33m'; b='\033[34m'; c='\033[36m'; d='\033[2m'; B='\033[1m'; n='\033[0m'
case "$AKYBA_PANE_ID" in
1)
printf "${d}\$${n} pnpm test --filter @acme/money\n\n"
printf " ${g}✓${n} src/format.test.ts ${d}(6 tests)${n} ${y}4ms${n}\n"
printf " ${g}✓${n} src/tax.test.ts ${d}(9 tests)${n} ${y}7ms${n}\n"
printf "   ${g}✓${n} vat rounds half up on .5 cents\n"
printf " ${g}✓${n} src/currency.test.ts ${d}(4 tests)${n} ${y}3ms${n}\n\n"
printf " ${d}Tests${n}  ${g}${B}19 passed${n} ${d}(19)${n}   ${d}Duration${n} 412ms\n\n"
printf "${d}\$${n} pnpm test --filter web\n\n"
printf " ${g}✓${n} src/cart/total.test.ts ${d}(5 tests)${n}\n"
printf " ${y}❯${n} src/checkout/Checkout.test.tsx ${d}(3 tests | 1 running)${n}\n"
;;
2)
printf "${d}\$${n} pnpm build --filter web\n"
printf " ${g}✓${n} built in 2.4s\n\n"
printf "${y}${B}?${n} ${B}Deploy which branch to staging?${n}\n\n"
printf "  ${c}❯ fix/vat-rounding${n}  ${d}round vat half up${n}\n"
printf "    main              ${d}a seed script${n}\n"
printf "    feat/cart         ${d}merged${n}\n\n"
printf "${d}  ↑↓ choose · enter confirm${n}\n"
;;
3)
printf "${d}\$${n} git diff --stat main\n"
printf " apps/web/src/catalog/Filter.tsx  | ${g}48 ++++++++${n}\n"
printf " apps/web/src/orders/List.tsx     | ${g}10 ++${n}${r}-${n}\n"
printf " apps/web/src/customers/List.tsx  | ${g}10 ++${n}${r}-${n}\n"
printf " packages/catalog/src/filter.ts   | ${g}36 +++++++${n}\n"
printf " 4 files changed, ${g}100 insertions(+)${n}, ${r}4 deletions(-)${n}\n\n"
printf "${g}●${n} The filter is on every list page: catalog,\n  orders and customers share one ${b}filter.ts${n}.\n\n"
printf "  ${g}✓${n} typecheck  ${g}✓${n} test  ${g}✓${n} lint\n"
;;
4)
printf "${d}\$${n} pnpm prisma migrate dev --name refunds\n\n"
printf "Prisma schema loaded from ${B}prisma${n}\n"
printf "Datasource ${B}\"db\"${n}: PostgreSQL ${d}\"acme\" at localhost:5432${n}\n\n"
printf "Applying migration ${c}\`20261018_refunds\`${n}\n\n"
printf "  ${g}+${n} CREATE TABLE \"Refund\"\n"
printf "  ${g}+${n} ALTER TABLE \"Payment\" ADD \"refundedCents\"\n"
printf "  ${y}…${n} generating the client\n"
;;
esac
