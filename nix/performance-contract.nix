# Evaluate the real VM producers without realizing their measurement outputs.
# Run with:
# nix eval --impure --expr 'let flake = builtins.getFlake (toString ./.); in import ./nix/performance-contract.nix { inherit (flake.performanceLib.${builtins.currentSystem}) mkPerformanceProducer; }'
{ mkPerformanceProducer }:
let
  selections = builtins.concatLists (map (backend:
    [ { inherit backend; producerKind = "storage"; } ]
    ++ map (browser: { inherit backend browser; producerKind = "browser"; })
      [ "chromium" "firefox" ]
  ) [ "sqlite" "postgres" ]);
  evaluate = selection:
    let
      producer = mkPerformanceProducer (selection // {
        freshnessNonce = "evaluation-contract";
        profile = "small";
      });
    in
    builtins.isString producer.drvPath && producer.drvPath != "";
in
assert builtins.all evaluate selections;
true
