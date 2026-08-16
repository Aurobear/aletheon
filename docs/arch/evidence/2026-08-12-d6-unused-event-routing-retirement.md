# D6 unused event-routing policy retirement

- `RouteAction` and `RoutingPolicy` had no caller outside their defining file;
  their three tests exercised an isolated policy that production never invoked.
- D6 deleted the module and its inventory/census rows rather than moving dead
  policy into Runtime.
- Repository-wide Rust symbol discovery is now zero for both symbols.
