-- Names for the Body Scan's percentage body-composition codes, confirmed by matching the
-- stored values of one weigh-in against the Withings app's Body Composition screen:
--   93 = Muscle %, 94 = Bone %, 95 = Water %, 122 = Lean Mass %, 170 = Visceral Fat (/20).
--
-- 122 and 170 correct two labels 0001 seeded wrong (122 is lean-mass %, not visceral fat;
-- 170 is the visceral-fat /20 index the app simply calls "Visceral Fat"). The remaining
-- codes (177, 178, 189, 201, 202, 203, 207) are not shown on that screen and stay
-- unlabelled rather than guessed; the measure_value view renders them as `type <code>`.

INSERT INTO measure_type (code, name, unit_label) VALUES
  (93,  'muscle_percent',    '%'),
  (94,  'bone_percent',      '%'),
  (95,  'water_percent',     '%'),
  (122, 'lean_mass_percent', '%'),
  (170, 'visceral_fat',      '/20')
ON CONFLICT (code) DO UPDATE
  SET name = EXCLUDED.name, unit_label = EXCLUDED.unit_label;
