# Jaunder

## Overview

`Jaunder` is an easily-hosted, multi-protocol social media application written in Rust. It provides a unified approach to consuming content from federated social networks (ActivityPub, RSS, Atom, JSON Feed, and Authenticated Transfer Protocol (AT Protocol / Bluesky)) and publishing original content (short-form, long-form, media) that is also pushed to those networks in whatever manner is appropriate. The primary design goals are:

- **Low operational cost**: Runs on a Raspberry Pi or small VPS as a single binary.
- **Privacy by default**: User data is never shared between users, even on multi-user instances.
- **Open standards**: All producer and consumer behavior is built on open, publicly specified protocols — including W3C/IETF standards (ActivityPub, RSS & Atom, WebSub, WebMentions) and significant open ecosystem protocols (AT Protocol, JSON Feed). The architecture supports incorporating additional protocols over time.
- **High fidelity**: Raw protocol data is stored without alteration alongside a pre-processed normalized form. The raw copy enables reprocessing if normalization logic changes; the processed copy makes reads fast.
- **Easy setup**: Users should be guided through the process of setting up the system, and wherever possible, the system should validate their settings and make suggestions if things seem wrong.
- **Easy maintenance**: Good practices should be the default; for instance, backups should be annoying until they're setup.

## Setting up jaunder

`Jaunder` is intended to be easy to get set up. To bring up the service by hand, one would do:

```
jaunder init # initial, guided database setup
jaunder serve
```

Additional configuration can then be done via the web interface.

By default, `Jaunder` will listen on http://localhost:3000/. To make this publically accessible, you need to have a reverse proxy (`Caddy` is recommended) that will listen on a publically accessible IP address and handle HTTPS.

If you are deploying with NixOS, import the shared module and enable the service:

```nix
{
  imports = [ inputs.jaunder.nixosModules.jaunder ];

  services.jaunder.enable = true;
  services.jaunder.bind = "0.0.0.0:3000";
}
```

For information on the design of `Jaunder`, see [the Design document](./docs/DESIGN.md).
