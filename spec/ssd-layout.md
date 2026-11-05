# SSD layout (portable mode)

```
/FIELDKB/
  launch-windows.exe  launch-macos.app  launch-linux
  bin/<os>-<arch>/<backend>/llama-server[.exe]   backends: metal | cuda | vulkan | cpu
  bin/<os>-<arch>/kb[.exe]  app binaries
  models/models.json                              catalog: id, file, min_ram_gb, context, license, safety_approved
  models/*.gguf
  packages/<id>@<version>/                        signed packages (format: package-format.md)
  trust/*.pub                                     publisher public keys
  private/                                        portable vault (ADR 0007)
  updates/                                        staged bundles and previous versions
  LICENSES/                                       notices and GPL source offers
```

`<os>` is `macos`, `windows` or `linux`; `<arch>` is Rust's `std::env::consts::ARCH` (`aarch64`, `x86_64`).
