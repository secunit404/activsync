"""Hevy ``exercise_template_id`` -> Strava ``exercise_type``.

Keyed by template id, so it holds regardless of the user's Hevy language.
Values are ids from `strava_exercises.EXERCISE_TYPES`: the same movement and
equipment where Strava has it, else the plain movement, else the group's
``*_GENERIC``. None marks a template with no sensible Strava exercise.
"""

from __future__ import annotations

TEMPLATE_TO_STRAVA: dict[str, str | None] = {
    "3BC06AD3": "TWENTY_ONES_BICEP_CURL",  # 21s Bicep Curl
    "B4F2FF72": "FLUTTER_KICKS",  # Ab Scissors
    "99D5F10E": "AB_WHEEL_ROLLOUT",  # Ab Wheel
    "5E0DDACE": "CARDIO_GENERIC",  # Aerobics
    "43573BB8": "ASSAULT_BIKE",  # Air Bike
    "A69FF221": "ARNOLD_PRESS",  # Arnold Press (Dumbbell)
    "D4A2FE7E": "FLYE_GENERIC",  # Around The World
    "5BFF35BA": "ASSISTED_PISTOL_SQUATS",  # Assisted Pistol Squats
    "4F5866F8": "BACK_EXTENSION",  # Back Extension (Hyperextension)
    "A05C064D": "MACHINE_BACK_EXTENSION",  # Back Extension (Machine)
    "091737FA": "BACK_EXTENSION",  # Back Extension (Weighted Hyperextension)
    "F51D9080": "BALL_SLAMS",  # Ball Slams
    "E8D86EE8": "BAND_PULLAPARTS",  # Band Pullaparts
    "084A67CA": "BATTLE_ROPES",  # Battle Ropes
    "B3B1D947": "BEAR_CRAWL",  # Bear Crawl
    "DDB29047": "BARBELL_BEHIND_THE_BACK_BICEP_WRIST_CURL",  # Behind the Back Bicep Wrist Curl (Barbell)
    "2DBCA395": "BAYESIAN_CURL",  # Behind the Back Curl (Cable)
    "3D1CDC75": "BELT_SQUAT",  # Belt Squat (Machine)
    "CD6DC8E5": "BENCH_DIP",  # Bench Dip
    "79D0BB3A": "BARBELL_BENCH_PRESS",  # Bench Press (Barbell)
    "99C1F2AD": "CHEST_PRESS",  # Bench Press (Cable)
    "3601968B": "DUMBBELL_BENCH_PRESS",  # Bench Press (Dumbbell)
    "0FBF7195": "BENCH_PRESS_GENERIC",  # Bench Press (Smith Machine)
    "35B51B87": "CLOSE_GRIP_BARBELL_BENCH_PRESS",  # Bench Press - Close Grip (Barbell)
    "E644F828": "WIDE_GRIP_BARBELL_BENCH_PRESS",  # Bench Press - Wide Grip (Barbell)
    "EA820646": "BENT_OVER_ROW",  # Bent Over Row (Band)
    "55E6546F": "BENT_OVER_BARBELL_ROW",  # Bent Over Row (Barbell)
    "23E92538": "BENT_OVER_DUMBBELL_ROW",  # Bent Over Row (Dumbbell)
    "EB813C91": "SMITH_MACHINE_ROW",  # Bent Over Row (Smith Machine)
    "A5AC6449": "BARBELL_BICEPS_CURL",  # Bicep Curl (Barbell)
    "ADA8623C": "CABLE_BICEPS_CURL",  # Bicep Curl (Cable)
    "37FCC2BB": "STANDING_DUMBBELL_BICEPS_CURL",  # Bicep Curl (Dumbbell)
    "AF328E3D": "MACHINE_BICEP_CURL",  # Bicep Curl (Machine)
    "0B4C1902": "SUSPENSION_BICEP_CURL",  # Bicep Curl (Suspension)
    "A41C7261": "BICYCLE_CRUNCH",  # Bicycle Crunch
    "594450D2": "BICYCLE_CRUNCH_RAISED_LEGS",  # Bicycle Crunch Raised Legs
    "BD0AD077": "BIRD_DOG",  # Bird Dog
    "56092DD1": "BOX_JUMP",  # Box Jump
    "38FC1AB9": "BARBELL_BOX_SQUAT",  # Box Squat (Barbell)
    "A0D8FA1B": "CARDIO_GENERIC",  # Boxing
    "B5D3A742": "DUMBBELL_BULGARIAN_SPLIT_SQUATS",  # Bulgarian Split Squat
    "0F24286A": "BARBELL_BULGARIAN_SPLIT_SQUAT",  # Bulgarian Split Squat (Barbell)
    "BB792A36": "BURPEE",  # Burpee
    "115CC72C": "BURPEE_BROAD_JUMP",  # Burpee Broad Jumps
    "86B00DDE": "BURPEE_OVER_THE_BAR",  # Burpee Over the Bar
    "9DCE2D64": "PEC_DECK_BUTTERFLY",  # Butterfly (Pec Deck)
    "CC55119B": "PALLOF_PRESS",  # Cable Core Pallof Press
    "23A48484": "CABLE_CRUNCH",  # Cable Crunch
    "651F844C": "CABLE_CROSSOVER",  # Cable Fly Crossovers
    "8C331CD8": "HIP_SWING_GENERIC",  # Cable Pull Through
    "92D858EA": "DOWN_TO_UP_CABLE_TWIST",  # Cable Twist (Down to up)
    "A2D838BD": "UP_TO_DOWN_CABLE_TWIST",  # Cable Twist (Up to down)
    "47B9DF13": "MACHINE_CALF_EXTENSION",  # Calf Extension (Machine)
    "91237BDD": "MACHINE_CALF_PRESS",  # Calf Press (Machine)
    "6FCD7755": "CHEST_DIP",  # Chest Dip
    "E9E4089F": "ASSISTED_CHEST_DIP",  # Chest Dip (Assisted)
    "29472BE1": "WEIGHTED_CHEST_DIP",  # Chest Dip (Weighted)
    "DDB7C19F": "BAND_CHEST_FLY",  # Chest Fly (Band)
    "12017185": "DUMBBELL_CHEST_FLY",  # Chest Fly (Dumbbell)
    "78683336": "MACHINE_CHEST_FLY",  # Chest Fly (Machine)
    "720B0D70": "SUSPENSION_CHEST_FLY",  # Chest Fly (Suspension)
    "EAC7D9C5": "CHEST_PRESS",  # Chest Press (Band)
    "7EB3F7C3": "MACHINE_CHEST_PRESS",  # Chest Press (Machine)
    "914F3A96": "CHEST_SUPPORTED_ROW",  # Chest Supported Incline Row (Dumbbell)
    "B582299E": "INCLINE_REVERSE_FLY",  # Chest Supported Reverse Fly (Dumbbell)
    "6A8D3193": "T_BAR_ROW",  # Chest Supported T Bar Row
    "F21D5693": "FLOOR_Y_RAISE",  # Chest Supported Y Raise (Dumbbell)
    "29083183": "CLOSE_GRIP_CHIN_UP",  # Chin Up
    "D23C609B": "ASSISTED_CHIN_UP",  # Chin Up (Assisted)
    "023943F1": "WEIGHTED_CHIN_UP",  # Chin Up (Weighted)
    "CC016611": "CLAMS",  # Clamshell
    "10D76E8F": "CLAP_PUSH_UPS",  # Clap Push Ups
    "ABB00838": "CLEAN",  # Clean
    "9E09CEC3": "CLEAN_AND_JERK",  # Clean and Jerk
    "D3095577": "DOUBLE_ARM_CLEAN_AND_PRESS_BARBELL",  # Clean and Press
    "652FEA39": "BARBELL_HANG_PULL",  # Clean Pull
    "E23F1F2B": "CARDIO_GENERIC",  # Climbing
    "724CDE60": "CONCENTRATION_CURL",  # Concentration Curl
    "32C4D4A2": "CROSS_BODY_HAMMER_CURL",  # Cross Body Hammer Curl
    "DCF3B31B": "CRUNCH",  # Crunch
    "EB43ADD4": "AB_CRUNCH_MACHINE",  # Crunch (Machine)
    "D928C232": "CRUNCH",  # Crunch (Weighted)
    "D39EC9EB": "CURTSY_LUNGE",  # Curtsy Lunge (Dumbbell)
    "D8F7F851": "CARDIO_GENERIC",  # Cycling
    "D8911FC4": "DEADBUG",  # Dead Bug
    "B9380898": "DEAD_HANG",  # Dead Hang
    "99507114": "CONVENTIONAL_DEADLIFTS",  # Deadlift (Band)
    "C6272009": "BARBELL_DEADLIFT",  # Deadlift (Barbell)
    "5F4E6DD3": "DUMBBELL_DEADLIFT",  # Deadlift (Dumbbell)
    "20870ED5": "CONVENTIONAL_DEADLIFTS",  # Deadlift (Smith Machine)
    "B923B230": "TRAP_BAR_DEADLIFT",  # Deadlift (Trap bar)
    "FA5D0DE1": "BARBELL_HIGH_PULL",  # Deadlift High Pull
    "DA0F0470": "DECLINE_DUMBBELL_BENCH_PRESS",  # Decline Bench Press (Barbell)
    "18487FA7": "DECLINE_DUMBBELL_BENCH_PRESS",  # Decline Bench Press (Dumbbell)
    "FAF31231": "MACHINE_DECLINE_BENCH_PRESS",  # Decline Bench Press (Machine)
    "FFC106CB": "MACHINE_DECLINE_BENCH_PRESS",  # Decline Bench Press (Smith Machine)
    "A351AED7": "DUMBBELL_CHEST_FLY",  # Decline Chest Fly (Dumbbell)
    "BC10A922": "DECLINE_CRUNCH",  # Decline Crunch
    "B2398CD1": "WEIGHTED_DECLINE_CRUNCH",  # Decline Crunch (Weighted)
    "C43825EA": "DECLINE_PUSH_UP",  # Decline Push Up
    "6575F52D": "DIAMOND_PUSH_UP",  # Diamond Push Up
    "7DA843A3": "STRETCH",  # Downward Dog
    "D950429E": "DRAG_CURL",  # Drag Curl
    "AFC29472": "DRAGON_FLAG",  # Dragon Flag
    "B7192800": "DRAGON_FLAG",  # Dragonfly
    "F1E57334": "DUMBBELL_ROW",  # Dumbbell Row
    "F3717B0E": "SINGLE_ARM_DUMBBELL_SNATCH",  # Dumbbell Snatch
    "F72FA239": "DUMBBELL_SQUEEZE_PRESS",  # Dumbbell Squeeze Press
    "BF6ECE89": "STEP_UP",  # Dumbbell Step Up
    "A01BE509": "ELBOW_TO_KNEE",  # Elbow to Knee
    "3303376C": "CARDIO_GENERIC",  # Elliptical Trainer
    "01A35BF9": "STANDING_EZ_BAR_BICEPS_CURL",  # EZ Bar Biceps Curl
    "BE640BA0": "FACE_PULL",  # Face Pull
    "50C613D0": "FARMERS_WALK",  # Farmers Walk
    "867AC3B6": "BARBELL_FEET_UP_BENCH_PRESS",  # Feet Up Bench Press (Barbell)
    "2AF8B552": "FIRE_HYDRANTS",  # Fire Hydrants
    "D53F2886": "BARBELL_FLOOR_PRESS",  # Floor Press (Barbell)
    "756EE329": "DUMBBELL_FLOOR_PRESS",  # Floor Press (Dumbbell)
    "A57D38D5": "FLOOR_TRICEPS_DIP",  # Floor Triceps Dip
    "2FA77B28": "FLUTTER_KICKS",  # Flutter Kicks
    "207DC636": "BODY_WEIGHT_JUMP_SQUAT",  # Frog Jumps
    "F6948F17": "FROG_PUMP",  # Frog Pumps (Dumbbell)
    "0E608350": "FRONT_LEVER_HOLD",  # Front Lever Hold
    "30F03BF0": "FRONT_LEVER_RAISE",  # Front Lever Raise
    "47B036EF": "FRONT_RAISE",  # Front Raise (Band)
    "BD86EFD5": "FRONT_RAISE",  # Front Raise (Barbell)
    "DBB91A3C": "FRONT_RAISE",  # Front Raise (Cable)
    "8293E554": "FRONT_RAISE",  # Front Raise (Dumbbell)
    "796E8E2C": "FRONT_RAISE",  # Front Raise (Suspension)
    "5046D0A9": "BARBELL_FRONT_SQUAT",  # Front Squat
    "1283BBA6": "BARBELL_SQUAT",  # Full Squat
    "CDA23948": "GLUTE_BRIDGE",  # Glute Bridge
    "49C922A1": "BARBELL_HIP_THRUST_ON_FLOOR",  # Glute Bridge (Barbell)
    "68B83EE0": "GLUTE_HAM_RAISE",  # Glute Ham Raise
    "CBA02382": "MACHINE_GLUTE_KICKBACK",  # Glute Kickback (Machine)
    "987234AB": "GLUTE_KICKBACK_ON_FLOOR",  # Glute Kickback on Floor
    "3D0C7C75": "GOBLET_SQUAT",  # Goblet Squat
    "4180C405": "BARBELL_GOOD_MORNING",  # Good Morning (Barbell)
    "05487216": "GORILLA_ROW",  # Gorilla Row (Kettlebell)
    "AC5A4C58": "MACHINE_HACK_SQUAT",  # Hack Squat
    "1E42FD5F": "MACHINE_HACK_SQUAT",  # Hack Squat (Machine)
    "1D4B3D6B": "DUMBBELL_HAMMER_CURL",  # Hammer Curl (Band)
    "36E8F14E": "CABLE_HAMMER_CURL",  # Hammer Curl (Cable)
    "7E3BC8B6": "DUMBBELL_HAMMER_CURL",  # Hammer Curl (Dumbbell)
    "BE3615CF": "HANDSTAND_HOLD",  # Handstand Hold
    "90B04F96": "HANDSTAND_PUSH_UP",  # Handstand Push Up
    "BD4E7E53": "BARBELL_HANG_POWER_CLEAN",  # Hang Clean
    "F4E77594": "BARBELL_HANG_POWER_SNATCH",  # Hang Snatch
    "08590920": "HANGING_KNEE_RAISE",  # Hanging Knee Raise
    "F8356514": "HANGING_LEG_RAISE",  # Hanging Leg Raise
    "567FB505": "HEEL_TAPS",  # Heel Taps
    "BE89C631": "DUMBBELL_SQUEEZE_PRESS",  # Hex Press (Dumbbell)
    "C9CCB878": "HIGH_KNEES",  # High Knee Skips
    "150E076B": "HIGH_KNEES",  # High Knees
    "023947AB": "CARDIO_GENERIC",  # HIIT
    "1C34A172": "CARDIO_GENERIC",  # Hiking
    "C469EA70": "CABLE_HIP_ABDUCTION",  # Hip Abduction (Cable)
    "F4B4C6EE": "MACHINE_HIP_ABDUCTION",  # Hip Abduction (Machine)
    "22578A94": "STANDING_ADDUCTION",  # Hip Adduction (Cable)
    "8BEBFED6": "MACHINE_HIP_ADDUCTION",  # Hip Adduction (Machine)
    "92B8C7E1": "HIP_THRUST",  # Hip Thrust
    "D57C2EC7": "BARBELL_HIP_THRUST",  # Hip Thrust (Barbell)
    "DA5430FC": "DUMBBELL_HIP_THRUST",  # Hip Thrust (Dumbbell)
    "68CE0B9B": "MACHINE_HIP_THRUST",  # Hip Thrust (Machine)
    "291ABA92": "HIP_THRUST",  # Hip Thrust (Smith Machine)
    "970ADF87": "HOLLOW_ROCK",  # Hollow Rock
    "50DFDFAB": "INCLINE_BARBELL_BENCH_PRESS",  # Incline Bench Press (Barbell)
    "07B38369": "INCLINE_DUMBBELL_BENCH_PRESS",  # Incline Bench Press (Dumbbell)
    "3A6FA3D1": "SMITH_MACHINE_INCLINE_BENCH_PRESS",  # Incline Bench Press (Smith Machine)
    "D3E2AB55": "INCLINE_DUMBBELL_FLYE",  # Incline Chest Fly (Dumbbell)
    "FBF92739": "MACHINE_INCLINE_CHEST_PRESS",  # Incline Chest Press (Machine)
    "39C99849": "INCLINE_PUSH_UP",  # Incline Push Ups
    "425805F4": "INVERTED_ROW",  # Inverted Row
    "24706DCD": "MACHINE_ISOLATERAL_CHEST_PRESS",  # Iso-Lateral Chest Press (Machine)
    "BC3492DA": "MACHINE_ISOLATERAL_HIGH_ROW",  # Iso-Lateral High Row (Machine)
    "91FAFBA3": "MACHINE_SINGLE_ARM_SEATED_ROW",  # Iso-Lateral Low Row
    "AA1EB7D8": "MACHINE_SINGLE_ARM_SEATED_ROW",  # Iso-Lateral Row (Machine)
    "D410F649": "SWISS_BALL_JACKKNIFE",  # Jack Knife (Suspension)
    "32B0B590": "V_UP",  # Jackknife Sit Up
    "74103F24": "JM_PRESS",  # JM Press (Barbell)
    "040BA2E3": "JUMP_ROPE",  # Jump Rope
    "BA3DE542": "JUMP_SHRUG",  # Jump Shrug
    "70D4EBBF": "BODY_WEIGHT_JUMP_SQUAT",  # Jump Squat
    "991833C2": "JUMPING_JACKS",  # Jumping Jack
    "5CC07A1F": "ALTERNATING_JUMP_LUNGE",  # Jumping Lunge
    "55BA475A": "KETTLEBELL_AROUND_THE_WORLD",  # Kettlebell Around the World
    "F99C211D": "KETTLEBELL_CLEAN",  # Kettlebell Clean
    "4E239ED8": "KETTLEBELL_BICEPS_CURL",  # Kettlebell Curl
    "A127DA73": "GOBLET_SQUAT",  # Kettlebell Goblet Squat
    "9B13BA4B": "BARBELL_HIGH_PULL",  # Kettlebell High Pull
    "6433CD93": "SHOULDER_PRESS_GENERIC",  # Kettlebell Shoulder Press
    "89304423": "SINGLE_ARM_KETTLEBELL_SNATCH",  # Kettlebell Snatch
    "F8A0FCCA": "KETTLEBELL_SWING",  # Kettlebell Swing
    "5F8903BF": "TURKISH_GET_UP",  # Kettlebell Turkish Get Up
    "A91838C0": "PULL_UP_GENERIC",  # Kipping Pull Up
    "98237BA2": "LEG_RAISE_PARALLEL_BARS",  # Knee Raise Parallel Bars
    "D82EA543": "LAT_PULLDOWN",  # Kneeling Pulldown (band)
    "B74A95BB": "MODIFIED_PUSH_UP",  # Kneeling Push Up
    "6D54A050": "CORE_GENERIC",  # L-Sit Hold
    "923874CA": "CHOP_GENERIC",  # Landmine 180
    "D7D7FCCE": "LANDMINE_ROW",  # Landmine Row
    "983274DD": "LANDMINE_SQUAT_AND_PRESS",  # Landmine Squat and Press
    "D2FE7B2E": "LAT_PULLDOWN",  # Lat Pulldown (Band)
    "6A6C31A5": "LAT_PULLDOWN",  # Lat Pulldown (Cable)
    "473CF5B8": "LAT_PULLDOWN",  # Lat Pulldown (Machine)
    "4E5257DE": "CABLE_LAT_PULLDOWN_CLOSE_GRIP",  # Lat Pulldown - Close Grip (Cable)
    "EC02979E": "LATERAL_WALKS_WITH_BAND_AT_ANKLES",  # Lateral Band Walks
    "07EBC4DC": "LATERAL_BOX_JUMP",  # Lateral Box Jump
    "DC59D143": "SIDE_LYING_LEG_RAISE",  # Lateral Leg Raises
    "F2A37357": "LATERAL_LUNGE",  # Lateral Lunge
    "DF200976": "LATERAL_RAISE_GENERIC",  # Lateral Raise (Band)
    "BE289E45": "CABLE_LATERAL_RAISE",  # Lateral Raise (Cable)
    "422B08F1": "LATERAL_RAISE_GENERIC",  # Lateral Raise (Dumbbell)
    "D5D0354D": "LATERAL_RAISE_GENERIC",  # Lateral Raise (Machine)
    "D731CCA8": "COSSACK_SQUAT",  # Lateral Squat
    "75A4F6C4": "MACHINE_LEG_EXTENSION",  # Leg Extension (Machine)
    "C7973E0E": "MACHINE_LEG_PRESS",  # Leg Press (Machine)
    "0EB695C9": "MACHINE_LEG_PRESS",  # Leg Press Horizontal (Machine)
    "0482DA98": "LEG_RAISE_PARALLEL_BARS",  # Leg Raise Parallel Bars
    "293483AD": "LOW_CABLE_FLY_CROSSOVERS",  # Low Cable Fly Crossovers
    "8A2E6481": "SUSPENSION_LOW_ROW",  # Low Row (Suspension)
    "5E1A7777": "LUNGE_GENERIC",  # Lunge
    "6E6EE645": "BARBELL_LUNGE",  # Lunge (Barbell)
    "B537D09F": "DUMBBELL_WALKING_LUNGES",  # Lunge (Dumbbell)
    "BD5935CF": "LYING_KNEE_RAISE",  # Lying Knee Raise
    "B8127AD1": "LEG_CURL_GENERIC",  # Lying Leg Curl (Machine)
    "09C9F635": "LYING_STRAIGHT_LEG_RAISE",  # Lying Leg Raise
    "22FEDCA4": "NECK_TILTS",  # Lying Neck Curls
    "31B8DA0F": "NECK_TILTS",  # Lying Neck Curls (Weighted)
    "0910BD28": "NECK_TILTS",  # Lying Neck Extension
    "D6A73CF2": "NECK_TILTS",  # Lying Neck Extension (Weighted)
    "C732C341": "MEADOWS_ROW",  # Meadows Rows (Barbell)
    "F49E31D6": "MOUNTAIN_CLIMBER",  # Mountain Climber
    "9F9C164B": "MUSCLE_UP",  # Muscle Up
    "F60BDDF8": "NEGATIVE_PULL_UP",  # Negative Pull Up
    "108D7A14": "NORDIC_CURL",  # Nordic Hamstrings Curls
    "DBE341AA": "OBLIQUE_CRUNCH",  # Oblique Crunch
    "B140095E": "ONE_ARM_PUSH_UP",  # One Arm Push Up
    "582ADA23": "CABLE_OVERHEAD_CURL",  # Overhead Curl (Cable)
    "4C6721B9": "OVERHEAD_LUNGE",  # Overhead Dumbbell Lunge
    "54E60954": "OVERHEAD_PLATE_RAISE",  # Overhead Plate Raise
    "7B8D84E8": "OVERHEAD_BARBELL_PRESS",  # Overhead Press (Barbell)
    "6AC96645": "OVERHEAD_DUMBBELL_PRESS",  # Overhead Press (Dumbbell)
    "B09A1304": "SMITH_MACHINE_OVERHEAD_PRESS",  # Overhead Press (Smith Machine)
    "2CFED196": "OVERHEAD_SQUAT",  # Overhead Squat
    "B5EFBF9C": "CABLE_OVERHEAD_TRICEPS_EXTENSION",  # Overhead Triceps Extension (Cable)
    "FBEE8279": "BARBELL_PARTIAL_GLUTE_BRIDGE",  # Partial Glute Bridge (Barbell)
    "CE1054CE": "BARBELL_SQUAT",  # Pause Squat (Barbell)
    "018ADC12": "BENT_OVER_BARBELL_ROW",  # Pendlay Row (Barbell)
    "30E293E3": "PENDULUM_SQUAT",  # Pendulum Squat (Machine)
    "0EFE8162": "PIKE_PUSH_UP",  # Pike Pushup
    "EC2510CD": "CORE_GENERIC",  # Pilates
    "942BAD12": "DUMBBELL_PINWHEEL_CURL",  # Pinwheel Curl (Dumbbell)
    "3FF6A22E": "PISTOL_SQUAT",  # Pistol Squat
    "C6C9B8A0": "PLANK_HOLD",  # Plank
    "31436F5D": "PLANK_TO_PUSH_UP",  # Plank Pushup
    "9283ABD5": "PLATE_CURL",  # Plate Curl
    "DBF9273A": "PLATE_FRONT_RAISE",  # Plate Front Raise
    "982734D4": "PLATE_PRESS",  # Plate Press
    "C628D768": "BARBELL_POWER_CLEAN",  # Power Clean
    "E22F9358": "BARBELL_POWER_SNATCH",  # Power Snatch
    "4F942934": "EZ_BAR_PREACHER_CURL",  # Preacher Curl (Barbell)
    "FAB6EB2F": "EZ_BAR_PREACHER_CURL",  # Preacher Curl (Dumbbell)
    "1E9A6B8E": "PREACHER_CURL_MACHINE",  # Preacher Curl (Machine)
    "84A77566": "OLYMPIC_LIFT_GENERIC",  # Press Under
    "1B2B1E7C": "PULL_UP_GENERIC",  # Pull Up
    "2C37EC5E": "MACHINE_ASSISTED_PULL_UP",  # Pull Up (Assisted)
    "56808FD2": "PULL_UP_GENERIC",  # Pull Up (Band)
    "729237D1": "PULL_UP_GENERIC",  # Pull Up (Weighted)
    "67280085": "DUMBBELL_PULLOVER",  # Pullover (Dumbbell)
    "B123DD01": "MACHINE_PULLOVER",  # Pullover (Machine)
    "542F3CD5": "PUSH_PRESS",  # Push Press
    "392887AA": "PUSH_UP_GENERIC",  # Push Up
    "19372ABC": "PUSH_UP_GENERIC",  # Push Up (Weighted)
    "947DAC23": "DIAMOND_PUSH_UP",  # Push Up - Close Grip
    "FE389074": "RACK_PULL",  # Rack Pull
    "C315DC2A": "CABLE_REAR_DELT_REVERSE_FLY",  # Rear Delt Reverse Fly (Cable)
    "E5988A0A": "DUMBBELL_REAR_DELT_FLY",  # Rear Delt Reverse Fly (Dumbbell)
    "D8281C62": "MACHINE_REAR_DELT_REVERSE_FLY",  # Rear Delt Reverse Fly (Machine)
    "1ADF8723": "MACHINE_GLUTE_KICKBACK",  # Rear Kick (Machine)
    "7FD2EC3E": "CARDIO_GENERIC",  # Recumbent Bike
    "1B89CA1B": "RENEGADE_ROW",  # Renegade Row (Dumbbell)
    "7952B5CD": "REVERSE_CRUNCH",  # Reverse Crunch
    "112FC6B7": "BARBELL_REVERSE_CURL",  # Reverse Curl (Barbell)
    "9F48F858": "REVERSE_CABLE_CURLS",  # Reverse Curl (Cable)
    "B567DD46": "BARBELL_REVERSE_CURL",  # Reverse Curl (Dumbbell)
    "9264ADA1": "CABLE_REAR_DELT_REVERSE_FLY",  # Reverse Fly Single Arm (Cable)
    "1FF4097B": "CONCENTRATION_CURL",  # Reverse Grip Concentration Curl
    "046E25A2": "UNDERHAND_LAT_PULLDOWN",  # Reverse Grip Lat Pulldown (Cable)
    "8B5BED30": "CABLE_TRICEPS_PUSHDOWN",  # Reverse Grip Triceps Pushdown
    "7C0028C3": "REVERSE_HYPER",  # Reverse Hyperextension
    "C284D923": "REVERSE_LUNGE",  # Reverse Lunge
    "818BA121": "BARBELL_REVERSE_LUNGE",  # Reverse Lunge (Barbell)
    "FFDA283B": "DUMBBELL_REVERSE_LUNGE",  # Reverse Lunge (Dumbbell)
    "99B49684": "REVERSE_PLANK",  # Reverse Plank
    "D8460FA6": "DUMBBELL_REVERSE_WRIST_CURL",  # Reverse Wrist Curl (Dumbbell)
    "51A0EDAA": "RING_DIP",  # Ring Dips
    "5F09F0FC": "RING_PULL_UP",  # Ring Pull Up
    "AEA56BDC": "PUSH_UP_GENERIC",  # Ring Push Up
    "2B4B7310": "BARBELL_ROMANIAN_DEADLIFT",  # Romanian Deadlift (Barbell)
    "72CFFAD5": "DUMBBELL_ROMANIAN_DEADLIFTS",  # Romanian Deadlift (Dumbbell)
    "0B9B92BA": "ROMANIAN_DEADLIFTS",  # Romanian Deadlift (Smith Machine)
    "234897AB": "ROPE_CABLE_CURL",  # Rope Cable Curl
    "9273BA17": "STRAIGHT_ARM_PULLDOWN",  # Rope Straight Arm Pulldown
    "0222DB42": "ROWING_MACHINE",  # Rowing Machine
    "AC1BB830": "RUNNING",  # Running
    "BB83BDDE": "RUSSIAN_TWIST",  # Russian Twist (Bodyweight)
    "2982AA23": "RUSSIAN_TWIST",  # Russian Twist (Weighted)
    "C7AE420A": "PULL_UP_GENERIC",  # Scapular Pull Ups
    "DF3BDB9C": "SEAL_ROW",  # Seal Row (Barbell)
    "7E93E23F": "SEAL_ROW",  # Seal Row (Dumbbell)
    "F1D60854": "SEATED_CABLE_ROW",  # Seated Cable Row - Bar Grip
    "C3BCABB3": "SEATED_CABLE_ROW",  # Seated Cable Row - Bar Wide Grip
    "0393F233": "SEATED_CABLE_ROW",  # Seated Cable Row - V Grip (Cable)
    "062AB91A": "SEATED_CALF_RAISE",  # Seated Calf Raise
    "6B4C797E": "CABLE_CROSSOVER",  # Seated Chest Flys (Cable)
    "5122E7D9": "SEATED_DIP_MACHINE",  # Seated Dip Machine
    "8BAB2735": "INCLINE_DUMBBELL_BICEPS_CURL",  # Seated Incline Curl (Dumbbell)
    "C01F58D1": "DUMBBELL_HAMMER_CURL",  # Seated Incline Hammer Curl (Dumbbell)
    "9372FFAA": "SEATED_DUMBBELL_LATERAL_RAISE",  # Seated Lateral Raise (Dumbbell)
    "11A123F3": "MACHINE_LEG_CURL_SEATED",  # Seated Leg Curl (Machine)
    "91AF29E0": "SEATED_BARBELL_PRESS",  # Seated Overhead Press (Barbell)
    "9930DF71": "SEATED_DUMBBELL_SHOULDER_PRESS",  # Seated Overhead Press (Dumbbell)
    "1006DF48": "SEATED_PALMS_UP_WRIST_CURL",  # Seated Palms Up Wrist Curl
    "1DF4A847": "MACHINE_SEATED_ROW",  # Seated Row (Machine)
    "9237BAD1": "MACHINE_SEATED_SHOULDER_PRESS",  # Seated Shoulder Press (Machine)
    "234BC743": "SEATED_TRICEPS_PRESS",  # Seated Triceps Press
    "95F2E076": "BARBELL_WRIST_CURL",  # Seated Wrist Curl (Barbell)
    "9202CC23": "BARBELL_SEATED_WRIST_EXTENSION",  # Seated Wrist Extension (Barbell)
    "878CD1D0": "SEATED_DUMBBELL_SHOULDER_PRESS",  # Shoulder Press (Dumbbell)
    "059E835D": "MACHINE_SEATED_SHOULDER_PRESS",  # Shoulder Press (Machine Plates)
    "502FCE31": "PLANK_SHOULDER_TAP",  # Shoulder Taps
    "0B841777": "BARBELL_SHRUG",  # Shrug (Barbell)
    "FFABC123": "SHRUG_GENERIC",  # Shrug (Cable)
    "ABEC557F": "DUMBBELL_SHRUG",  # Shrug (Dumbbell)
    "19A38071": "SHRUG_GENERIC",  # Shrug (Machine)
    "742E5BD5": "SHRUG_GENERIC",  # Shrug (Smith Machine)
    "37F9EDA7": "SIDE_BEND",  # Side Bend
    "026FD047": "DUMBBELL_SIDE_BEND",  # Side Bend (Dumbbell)
    "E3EDA509": "SIDE_PLANK",  # Side Plank
    "8372BBCC": "CABLE_CROSSOVER",  # Single Arm Cable Crossover
    "D0C4A899": "SEATED_CABLE_ROW",  # Single Arm Cable Row
    "B33B526E": "CABLE_BICEPS_CURL",  # Single Arm Curl (Cable)
    "1837BA23": "STANDING_SINGLE_ARM_SHOULDER_PRESS",  # Single Arm Landmine Press (Barbell)
    "2EE45F81": "SINGLE_ARM_LAT_PULLDOWN",  # Single Arm Lat Pulldown
    "DE68C825": "CABLE_LATERAL_RAISE",  # Single Arm Lateral Raise (Cable)
    "8347DFD1": "DUMBBELL_SINGLE_ARM_TRICEP_EXTENSION",  # Single Arm Tricep Extension (Dumbbell)
    "552AB030": "CABLE_SINGLE_ARM_TRICEPS_PUSHDOWN",  # Single Arm Triceps Pushdown (Cable)
    "629AE73D": "MACHINE_SINGLE_LEG_QUAD_EXTENSION",  # Single Leg Extensions
    "CBC0D834": "SINGLE_LEG_GLUTE_BRIDGE",  # Single Leg Glute Bridge
    "487B3755": "SINGLE_LEG_HIP_RAISE",  # Single Leg Hip Thrust
    "D1CD146F": "DUMBBELL_SINGLE_LEG_HIP_THRUST",  # Single Leg Hip Thrust (Dumbbell)
    "3FD83744": "MACHINE_SINGLE_LEG_PRESS",  # Single Leg Press (Machine)
    "93472AC1": "SINGLE_LEG_BARBELL_ROMANIAN_DEADLIFTS",  # Single Leg Romanian Deadlift (Barbell)
    "937292AB": "SINGLE_LEG_DUMBBELL_ROMANIAN_DEADLIFTS",  # Single Leg Romanian Deadlift (Dumbbell)
    "238BA231": "SINGLE_LEG_STANDING_CALF_RAISE",  # Single Leg Standing Calf Raise
    "E51CCBE5": "SINGLE_LEG_BARBELL_CALF_RAISE",  # Single Leg Standing Calf Raise (Barbell)
    "5DA40761": "SINGLE_LEG_DUMBBELL_STANDING_CALF_RAISE",  # Single Leg Standing Calf Raise (Dumbbell)
    "B05C2C29": "SINGLE_LEG_STANDING_CALF_RAISE",  # Single Leg Standing Calf Raise (Machine)
    "F5DEF1EB": "SISSY_SQUAT",  # Sissy Squat (Weighted)
    "022DF610": "SIT_UP_GENERIC",  # Sit Up
    "9237BA12": "SIT_UP_GENERIC",  # Sit Up (Weighted)
    "24A809EF": "CARDIO_GENERIC",  # Skating
    "5D99A2FA": "SKI_ERG",  # Ski Erg
    "84325755": "CARDIO_GENERIC",  # Skiing
    "875F585F": "SKULL_CRUSHER",  # Skullcrusher (Barbell)
    "68F8A292": "DUMBBELL_SKULLCRUSHER",  # Skullcrusher (Dumbbell)
    "7D1BD41D": "SLED_PULL",  # Sled Pull
    "7757171F": "SLED_PUSH",  # Sled Push
    "FB09C938": "BARBELL_SNATCH",  # Snatch
    "911A58D3": "CARDIO_GENERIC",  # Snowboarding
    "2348AB72": "SPIDER_CURL",  # Spider Curl (Barbell)
    "90427D4A": "SPIDER_CURL",  # Spider Curl (Dumbbell)
    "C10A5AC9": "PLANK_GENERIC",  # Spiderman
    "213AB238": "CARDIO_GENERIC",  # Spinning
    "E764B907": "SPLIT_JERK",  # Split Jerk
    "20C1A3CB": "DUMBBELL_SPLIT_SQUAT",  # Split Squat (Dumbbell)
    "9283BABA": "RUNNING",  # Sprints
    "F1258206": "SQUAT_GENERIC",  # Squat (Band)
    "D04AC939": "BARBELL_SQUAT",  # Squat (Barbell)
    "9694DA61": "AIR_SQUAT",  # Squat (Bodyweight)
    "DCFF3E9F": "DUMBBELL_SQUAT",  # Squat (Dumbbell)
    "CC35A01F": "SQUAT_GENERIC",  # Squat (Machine)
    "DDCC3821": "SMITH_MACHINE_SQUAT",  # Squat (Smith Machine)
    "B82D6418": "SQUAT_GENERIC",  # Squat (Suspension)
    "A8814C06": "ROW_GENERIC",  # Squat Row
    "4377A52B": "STAIRMASTER",  # Stair Machine (Floors)
    "4377A52C": "STAIRMASTER",  # Stair Machine (Steps)
    "ACB2751D": "CABLE_KICKBACK",  # Standing Cable Glute Kickbacks
    "06745E58": "STANDING_CALF_RAISE",  # Standing Calf Raise
    "E53CCBE5": "STANDING_CALF_RAISE",  # Standing Calf Raise (Barbell)
    "6DA40660": "STANDING_CALF_RAISE",  # Standing Calf Raise (Dumbbell)
    "E05C2C38": "STANDING_CALF_RAISE",  # Standing Calf Raise (Machine)
    "AA52E8D2": "STANDING_CALF_RAISE",  # Standing Calf Raise (Smith)
    "6120CAAB": "STANDING_LEG_CURL",  # Standing Leg Curls
    "073032BB": "STANDING_BARBELL_PRESS",  # Standing Military Press (Barbell)
    "28927A36": "SHOULDER_STABILITY_GENERIC",  # Standing Y Raise (Cable)
    "128A2381": "STEP_UP",  # Step Up
    "EE2938D1": "GIRONDA_STERNUM_PULL_UP",  # Sternum Pull up (Gironda)
    "D2387AB1": "STRAIGHT_ARM_PULLDOWN",  # Straight Arm Lat Pulldown (Cable)
    "2A48E443": "STRAIGHT_LEG_DEADLIFT",  # Straight Leg Deadlift
    "527DA061": "STRETCH",  # Stretching
    "7593855D": "SUITCASE_CARRY",  # Suitcase Carry (Dumbbell)
    "D20D7BBE": "SUMO_DEADLIFT",  # Sumo Deadlift
    "3F5F8D40": "SUMO_SQUAT",  # Sumo Squat
    "6622E5A0": "SUMO_SQUAT",  # Sumo Squat (Barbell)
    "05293BCA": "SUMO_SQUAT",  # Sumo Squat (Dumbbell)
    "5E10D0E6": "SUMO_SQUAT",  # Sumo Squat (Kettlebell)
    "218DA87C": "SUPERMAN_FROM_FLOOR",  # Superman
    "B60A678F": "CARDIO_GENERIC",  # Swimming
    "08A2974E": "T_BAR_ROW",  # T Bar Row
    "90E506D5": "THRUSTERS",  # Thruster (Barbell)
    "10313AFD": "THRUSTERS",  # Thruster (Kettlebell)
    "75BAC5C3": "CORE_GENERIC",  # Toe Touch
    "B94E35E1": "TOES_TO_BAR",  # Toes to Bar
    "FBB62888": "CORE_GENERIC",  # Torso Rotation
    "243710DE": "CARDIO_GENERIC",  # Treadmill
    "28BB4A95": "TRICEP_DIP",  # Triceps Dip
    "4B4BF8C2": "TRICEP_DIP",  # Triceps Dip (Assisted)
    "10347BAC": "TRICEP_DIP",  # Triceps Dip (Weighted)
    "2F8D3067": "SEATED_BARBELL_OVERHEAD_TRICEPS_EXTENSION",  # Triceps Extension (Barbell)
    "21310F5F": "CABLE_OVERHEAD_TRICEPS_EXTENSION",  # Triceps Extension (Cable)
    "3765684D": "OVERHEAD_DUMBBELL_TRICEPS_EXTENSION",  # Triceps Extension (Dumbbell)
    "3092FADD": "MACHINE_TRICEP_EXTENSION",  # Triceps Extension (Machine)
    "6182EE83": "TRICEPS_EXTENSION_GENERIC",  # Triceps Extension (Suspension)
    "EC3B69A3": "DUMBBELL_KICKBACK",  # Triceps Kickback (Cable)
    "6127A3AD": "DUMBBELL_KICKBACK",  # Triceps Kickback (Dumbbell)
    "CDC472B1": "TRICEPS_PRESSDOWN",  # Triceps Pressdown
    "93A552C6": "CABLE_TRICEPS_PUSHDOWN",  # Triceps Pushdown
    "94B7239B": "CABLE_TRICEPS_PUSHDOWN",  # Triceps Rope Pushdown
    "7AB9A362": "BARBELL_UPRIGHT_ROW",  # Upright Row (Barbell)
    "286C1D0B": "CABLE_UPRIGHT_ROW",  # Upright Row (Cable)
    "797F0782": "KETTLEBELL_UPRIGHT_ROW",  # Upright Row (Dumbbell)
    "6BE68B62": "V_UP",  # V Up
    "72880C57": "LAT_PULLDOWN",  # Vertical Traction (Machine)
    "72297E8C": "STANDING_DUMBBELL_BICEPS_CURL",  # Waiter Curl (Dumbbell)
    "33EDD7DB": "CARDIO_GENERIC",  # Walking
    "A733CC5B": "DUMBBELL_WALKING_LUNGES",  # Walking Lunge (Dumbbell)
    "F30A4F01": "SANDBAG_LUNGE",  # Walking Lunge (Sandbag)
    "A1F47ACC": "WALL_BALL",  # Wall Ball
    "C8706C80": "WALL_SIT",  # Wall Sit
    "79EF4E4F": "WARM_UP_GENERIC",  # Warm Up
    "7C50F118": "WIDE_PULL_UP",  # Wide Pull Up
    "B31EB524": "DUMBBELL_WIDEELBOW_TRICEPS_PRESS",  # Wide-Elbow Triceps Press (Dumbbell)
    "0E4523F4": "WRIST_ROLLER",  # Wrist Roller
    "8C9D2928": "STRETCH",  # Yoga
    "40C6A9FC": "ZERCHER_SQUAT",  # Zercher Squat
    "123EE239": "DUMBBELL_ZOTTMAN_CURL",  # Zottman Curl (Dumbbell)
}
