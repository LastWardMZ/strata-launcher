-- Foto de perfil: WebP de 128×128 (máx. 24 KB) en base64. `avatar_v` cambia con cada foto o borrado y
-- permite a los launchers de tus amigos saber si tienen que descargarla otra vez sin pedirla siempre.
ALTER TABLE users ADD COLUMN avatar TEXT NOT NULL DEFAULT '';
ALTER TABLE users ADD COLUMN avatar_v INTEGER NOT NULL DEFAULT 0;
