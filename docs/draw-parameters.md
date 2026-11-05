# Draw parameters and wind regions

`msb::MapLayout::wind_sfx_region` reads a type-6 region as `WindSfxRegion` with the effect ID, wind-area index and retained unknown float, using each game's layout. Other region types return `None`.

`param::PartsDrawParam`, `AssetGeometryParam` and `DrawDistScaleParam` expose Elden Ring's texture-switch distances, asset-to-draw-param references and distance-scale rows; unsupported revisions and row lengths are errors.

`param::BulletParam` (through `game_params::read_bullet_param`) reads Elden Ring's bullets: their effects, life, speed, acceleration, gravity and the bullets they make, with each field's meaning as Paramdex describes it.

`gparam::Gparam::time` retains the stored time axis; `TimeAxis::sampling` gives the source offset and period for sampling at the caller's hour. Reading the fields does not choose a texture form or weather for the caller.
