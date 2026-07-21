# Bosch BMA42x feature configuration

`bma421.hex` and `bma425.hex` contain the 6,144-byte feature-engine
configuration streams used for PineTime's BMA421 and BMA425 variants.

The streams were extracted byte-for-byte from Bosch Sensor API file
`src/drivers/Bma421_C/bma423.c` as vendored by InfiniTime commit
`71d1f5b45bd2bc9499acb16a5b6893f5aae0a15f`. That file carries the Bosch
Sensortec BSD-3-Clause notice reproduced in `LICENSE`.

Source: <https://github.com/InfiniTimeOrg/InfiniTime/blob/71d1f5b45bd2bc9499acb16a5b6893f5aae0a15f/src/drivers/Bma421_C/bma423.c>

The text encoding is lowercase hexadecimal without separators. `build.rs`
validates and decodes each stream into `OUT_DIR`; only the 6,144 decoded bytes
per supported variant are linked into diagnostics firmware.
