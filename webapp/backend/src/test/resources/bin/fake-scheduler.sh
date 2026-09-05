#!/usr/bin/env bash
# Фейковый daily_scheduler для интеграционных тестов: печатает характерные строки прогресса
# (ровно те, что разбирает ProgressTracker), создаёт schedule.csv, report.json
# (printers[0].variable_batch_schedule) и plates.3mf по своим аргументам. Слайсер не вызывается.
set -euo pipefail

printer_spec=""
output_schedule="schedule.csv"
output_report="report.json"
bench_mode="adaptive"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --printer) printer_spec="$2"; shift 2;;
    --output-schedule) output_schedule="$2"; shift 2;;
    --output-report) output_report="$2"; shift 2;;
    --bench-mode) bench_mode="$2"; shift 2;;
    *) shift;;
  esac
done

printer_name="Bambu Lab A2L"
plates_3mf="plates.3mf"
# spec приходит одной строкой: profile=...,source=...,prep-min=10,plates-3mf=plates.3mf
IFS=',' read -ra spec_parts <<< "$printer_spec"
for part in "${spec_parts[@]}"; do
  case "$part" in
    plates-3mf=*) plates_3mf="${part#plates-3mf=}";;
  esac
done

echo "$printer_name: 30 objects"
echo "[$printer_name] slicing sample 1/5: batch 8/30"
echo "[$printer_name] slicing sample 3/5: batch 20/30"
echo "[$printer_name] slicing sample 5/5: batch 30/30"
echo "[$printer_name] refining: slicing batch 15 (2.1% curvature)"
echo "[$printer_name] 5 measured sizes cover batches 1..=30 (83% fewer slices than a full sweep)"

cat > "$output_schedule" <<'CSV'
printer,sequence,batch_size,start_seconds,end_seconds,start_clock,end_clock,slicer_seconds,preparation_seconds,filament_grams,cumulative_objects,overnight
Bambu Lab A2L,1,7,36000,44760,10:00,12:26,41160,600,210.5000,7,false
Bambu Lab A2L,2,7,45360,54120,12:36,15:02,41160,600,210.5000,14,false
Bambu Lab A2L,3,7,54720,63480,15:12,17:38,41160,600,210.5000,21,false
Bambu Lab A2L,4,5,64100,70000,17:48,19:26,34000,600,150.2500,26,false
Bambu Lab A2L,5,5,70600,76500,19:36,21:15,34000,600,150.2500,31,false
Bambu Lab A2L,6,10,77100,87900,21:25,D+1 00:25,49200,600,300.7500,41,true
Bambu Lab A2L,7,10,88500,99300,D+1 00:35,D+1 03:35,49200,600,300.7500,51,true
Bambu Lab A2L,8,12,99900,112500,D+1 03:45,D+1 07:15,60000,600,360.0000,63,true
CSV

cat > "$output_report" <<'JSON'
{
  "combined_objects_per_day": 63,
  "printers": [
    {
      "printer": "Bambu Lab A2L",
      "gain_vs_full_plate_only": 12,
      "variable_batch_schedule": {
        "printer": "Bambu Lab A2L",
        "total_objects": 63,
        "total_filament_grams": 1893.5,
        "completion_seconds": 112500,
        "jobs": [
          {"sequence": 1, "batch_size": 7,  "start_seconds": 36000,  "end_seconds": 44760,  "slicer_seconds": 41160, "preparation_seconds": 600, "filament_grams": 210.5,  "cumulative_objects": 7,  "overnight": false},
          {"sequence": 2, "batch_size": 7,  "start_seconds": 45360,  "end_seconds": 54120,  "slicer_seconds": 41160, "preparation_seconds": 600, "filament_grams": 210.5,  "cumulative_objects": 14, "overnight": false},
          {"sequence": 3, "batch_size": 7,  "start_seconds": 54720,  "end_seconds": 63480,  "slicer_seconds": 41160, "preparation_seconds": 600, "filament_grams": 210.5,  "cumulative_objects": 21, "overnight": false},
          {"sequence": 4, "batch_size": 5,  "start_seconds": 64100,  "end_seconds": 70000,  "slicer_seconds": 34000, "preparation_seconds": 600, "filament_grams": 150.25, "cumulative_objects": 26, "overnight": false},
          {"sequence": 5, "batch_size": 5,  "start_seconds": 70600,  "end_seconds": 76500,  "slicer_seconds": 34000, "preparation_seconds": 600, "filament_grams": 150.25, "cumulative_objects": 31, "overnight": false},
          {"sequence": 6, "batch_size": 10, "start_seconds": 77100,  "end_seconds": 87900,  "slicer_seconds": 49200, "preparation_seconds": 600, "filament_grams": 300.75, "cumulative_objects": 41, "overnight": true},
          {"sequence": 7, "batch_size": 10, "start_seconds": 88500,  "end_seconds": 99300,  "slicer_seconds": 49200, "preparation_seconds": 600, "filament_grams": 300.75, "cumulative_objects": 51, "overnight": true},
          {"sequence": 8, "batch_size": 12, "start_seconds": 99900,  "end_seconds": 112500, "slicer_seconds": 60000, "preparation_seconds": 600, "filament_grams": 360.0,  "cumulative_objects": 63, "overnight": true}
        ]
      },
      "full_plate_only_schedule": {
        "printer": "Bambu Lab A2L",
        "total_objects": 30,
        "total_filament_grams": 900.0,
        "completion_seconds": 100000,
        "jobs": []
      }
    }
  ],
  "benchmarks": [],
  "sensitivity": []
}
JSON

printf 'fake multi-plate 3mf for tests: %s\n' "$plates_3mf" > "$plates_3mf"

echo "$printer_name: writing 4 plate(s) to $plates_3mf"
echo ""
echo "Optimized 24-hour production cycle:"
echo "$printer_name: 63 objects, 8 jobs [7@10:00, 7@12:36, 7@15:12, 5@17:48, 5@19:36, 10@21:25, 10@D+1 00:35, 12@D+1 03:45], gain vs full plates: +12"
echo "Combined: 63 objects/day"
