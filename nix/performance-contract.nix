# Evaluate the real VM producers without realizing their measurement outputs.
# Run with:
# nix eval --impure --expr 'let flake = builtins.getFlake (toString ./.); in import ./nix/performance-contract.nix { inherit (flake.performanceLib.${builtins.currentSystem}) mkPerformanceProducer selectPerformanceProducers; }'
{ mkPerformanceProducer, selectPerformanceProducers }:
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
  selected = selectPerformanceProducers {
    freshnessNonce = "selection-contract";
    profile = "small";
    backends = "";
    browsers = "";
    workload = "all";
    posts = "";
    authors = "";
    revisions = "";
  };
  selectedNames = value: map (producer: producer.name) value.producers;
  allProfiles = map (profile: selectPerformanceProducers {
    freshnessNonce = "profile-${profile}-contract";
    inherit profile;
    backends = "";
    browsers = "";
    workload = "all";
  }) [ "small" "medium" "large" ];
  defaultNames = [
    "performance-storage-sqlite"
    "performance-browser-sqlite-chromium"
    "performance-storage-postgres"
    "performance-browser-postgres-chromium"
  ];
  explicitDefaults = selectPerformanceProducers {
    freshnessNonce = "explicit-default-contract";
    profile = "small";
    backends = "postgres,sqlite,postgres";
    browsers = "chromium,chromium";
    workload = "all";
  };
  storageOnly = selectPerformanceProducers {
    freshnessNonce = "storage-only-contract";
    profile = "small";
    backends = "sqlite,postgres";
    browsers = "";
    workload = "storage";
  };
  browserOnly = selectPerformanceProducers {
    freshnessNonce = "browser-only-contract";
    profile = "small";
    backends = "";
    browsers = "firefox";
    workload = "browser";
  };
  deduplicated = selectPerformanceProducers {
    freshnessNonce = "deduplicated-contract";
    profile = "small";
    backends = "postgres,sqlite,postgres";
    browsers = "firefox,chromium,chromium";
    workload = "all";
  };
  overridden = selectPerformanceProducers {
    freshnessNonce = "count-contract";
    profile = "small";
    backends = "sqlite";
    browsers = "";
    workload = "storage";
    posts = "120";
    authors = "12";
    revisions = "777";
  };
  invalidZeroCount = builtins.tryEval (
    builtins.deepSeq (selectPerformanceProducers {
      freshnessNonce = "invalid-zero-count";
      profile = "small";
      backends = "sqlite";
      browsers = "";
      workload = "storage";
      posts = "0";
    }) true
  );
  invalidStorageBrowser = builtins.tryEval (
    builtins.deepSeq (selectPerformanceProducers {
      freshnessNonce = "invalid-storage-browser";
      profile = "small";
      backends = "sqlite";
      browsers = "chromium";
      workload = "storage";
    }) true
  );
in
assert builtins.all evaluate selections;
assert selectedNames selected == defaultNames;
assert selectedNames explicitDefaults == defaultNames;
assert explicitDefaults.selection == selected.selection;
assert selected.profile == "small";
assert map (value: value.profile) allProfiles == [ "small" "medium" "large" ];
assert builtins.all (value: selectedNames value == defaultNames) allProfiles;
assert selected.selection.backends == [ "sqlite" "postgres" ];
assert selected.selection.browsers == [ "chromium" ];
assert selected.selection.storage && selected.selection.browser;
assert selectedNames storageOnly == [ "performance-storage-sqlite" "performance-storage-postgres" ];
assert storageOnly.selection.browsers == [ ];
assert selectedNames browserOnly == [ "performance-browser-sqlite-firefox" "performance-browser-postgres-firefox" ];
assert selectedNames deduplicated == [
  "performance-storage-sqlite"
  "performance-browser-sqlite-chromium"
  "performance-browser-sqlite-firefox"
  "performance-storage-postgres"
  "performance-browser-postgres-chromium"
  "performance-browser-postgres-firefox"
];
assert overridden.selection.count_overrides == {
  posts = 120;
  authors = 12;
  revisions = 777;
};
assert !invalidStorageBrowser.success;
assert !invalidZeroCount.success;
true
