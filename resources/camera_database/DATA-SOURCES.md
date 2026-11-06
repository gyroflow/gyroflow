# Camera catalog sources

`camera_database.json` is built from Lensfun at commit
`a5e2caeb6b646b7a0bb9ed76033b56f8c0499e86` and the explicitly sourced
identities in the producer registry. Lensfun is licensed under CC BY-SA 3.0;
its applicable license is included in `LENSFUN-DATABASE-LICENSE.txt`. The
output records the Lensfun revision and license plus the curated source URLs.

Gyroflow profiles only mark a unique match to an existing catalog identity.
They do not create catalog entries or contribute camera crop metadata or
profile-derived aliases. Reviewed historical camera-pair mappings remain
producer inputs and are not emitted as camera aliases.

The source registry and generator are maintained in the Gyroflow lens profile
repository: https://github.com/gyroflow/lens_profiles.

The catalog is a modified compilation of Lensfun data. Preserve this notice
when redistributing it.
