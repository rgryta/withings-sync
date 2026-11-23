-- Generic names for the Body Scan's raw internal signals. These codes are NOT shown as
-- labeled biomarkers anywhere in the Withings app; the scale records them and Withings'
-- algorithms derive the body-composition figures from them. Confirmed from the app's
-- impedance subsystem and body-balance curve analysis plus the stored value ranges
-- (177/178/189 ~= 400-550 ohms; 201/202/203 ~= small +/- balance parameters; 207 a flag).
-- These names are our own descriptions of the raw signals, not Withings' labels.

INSERT INTO measure_type (code, name, unit_label) VALUES
  (177, 'impedance_1',     'ohm'),
  (178, 'impedance_2',     'ohm'),
  (189, 'impedance_3',     'ohm'),
  (201, 'body_balance_1',  ''),
  (202, 'body_balance_2',  ''),
  (203, 'body_balance_3',  ''),
  (207, 'body_balance_state', '')
ON CONFLICT (code) DO UPDATE
  SET name = EXCLUDED.name, unit_label = EXCLUDED.unit_label;
