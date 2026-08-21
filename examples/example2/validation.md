# CPA-Manager-Plus 锁定验证

验证日期：2026-08-21

```sh
cargo run --quiet --manifest-path ../../Cargo.toml -- update
nix build --no-link -L \
  '/nix/store/dwb01an9z9z1gvnkap5p5rbldib801h0-cpa-manager-plus-manager-server-v1.12.1-go-modules.drv^out' \
  '/nix/store/b82c15xfknkkw0gq6b2m0rglchkz9878-cpa-manager-plus-web-v1.12.1-npm-deps.drv^out'
```

- upstream：`seakee/CPA-Manager-Plus` `v1.12.1`
- source：`sha256-tq5F5NgKyahsYOmv5NDF1TMwc5OfTx18aCd2PyrvTNM=`
- vendor：`sha256-OhtGqAXPPbTIqGB9VvQ5UTgCNt7fw2yNKkEIgWhGuiM=`
- npm deps：`sha256-JxY+dC/fpNpEe8lH2UwgjJbX6HVdgjaURvrQbxPfeDc=`
- Go FOD output：`/nix/store/nhzqb0xjri08vqajb1j7dagnvwlvfvfx-cpa-manager-plus-manager-server-v1.12.1-go-modules`
- npm FOD output：`/nix/store/3j4skvj92bcgbd30s6ljq789883w1nid-cpa-manager-plus-web-v1.12.1-npm-deps`

两项 FOD 均以锁定哈希构建成功，未再出现 hash mismatch。
