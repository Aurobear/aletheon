# E7 Mnemosyne embodied-episode cutover

- `EmbodiedEpisode` and `EpisodeAttempt` now live beside Mnemosyne's sole
  append-only repository and persisted schema reader/writer.
- Fabric's duplicate rich model, public rows, and census rows were deleted;
  no compatibility alias was added.
