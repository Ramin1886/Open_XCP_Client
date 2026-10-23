// SPDX-License-Identifier: GPL-2.0-only
//! Realistic embedded ADAS A2L definition (modeled after Vector `xcplite` 2.2.2 output)
//! containing nested `TYPEDEF_STRUCTURE` / `INSTANCE` declarations, multi-dimensional
//! `MATRIX_DIM` arrays, dot-separated measurements, multi-rate XCP DAQ events, and
//! scalar/enum/array `CHARACTERISTIC` calibration parameters.

/// Returns a complete, realistic ADAS A2L file string generated as by `xcplite` 2.2.2.
pub fn sample_adas_a2l_text() -> &'static str {
    r#"ASAP2_VERSION 1 71
/begin PROJECT OpenXcpAdasProject "Embedded ADAS Software on QNX 7.1 (xcplite 2.2.2)"

  /begin MODULE AdasCoreModule "Qualcomm SoC ADAS Perception & Motion Control"

    /begin IF_DATA XCP
      /begin DAQ
        DYNAMIC 0 2 0 OPTIMISATION_TYPE_DEFAULT ADDRESS_EXTENSION_FREE IDENTIFICATION_FIELD_TYPE_RELATIVE_BYTE GRANULARITY_ODT_ENTRY_SIZE_DAQ_BYTE 0xF8 OVERLOAD_INDICATION_NONE
        /begin TIMESTAMP_SUPPORTED
          0x01 SIZE_DWORD UNIT_1NS TIMESTAMP_FIXED
        /end TIMESTAMP_SUPPORTED
        /begin EVENT "Event_10ms" "Ev10ms" 0x0000 DAQ 0xFF 10 6 0x00 /end EVENT
        /begin EVENT "Event_50ms" "Ev50ms" 0x0001 DAQ 0xFF 50 6 0x00 /end EVENT
      /end DAQ
    /end IF_DATA

    /begin RECORD_LAYOUT R_UBYTE     FNC_VALUES 1 UBYTE        ROW_DIR DIRECT /end RECORD_LAYOUT
    /begin RECORD_LAYOUT R_SBYTE     FNC_VALUES 1 SBYTE        ROW_DIR DIRECT /end RECORD_LAYOUT
    /begin RECORD_LAYOUT R_UWORD     FNC_VALUES 1 UWORD        ROW_DIR DIRECT /end RECORD_LAYOUT
    /begin RECORD_LAYOUT R_SWORD     FNC_VALUES 1 SWORD        ROW_DIR DIRECT /end RECORD_LAYOUT
    /begin RECORD_LAYOUT R_ULONG     FNC_VALUES 1 ULONG        ROW_DIR DIRECT /end RECORD_LAYOUT
    /begin RECORD_LAYOUT R_SLONG     FNC_VALUES 1 SLONG        ROW_DIR DIRECT /end RECORD_LAYOUT
    /begin RECORD_LAYOUT R_UINT64    FNC_VALUES 1 A_UINT64     ROW_DIR DIRECT /end RECORD_LAYOUT
    /begin RECORD_LAYOUT R_INT64     FNC_VALUES 1 A_INT64      ROW_DIR DIRECT /end RECORD_LAYOUT
    /begin RECORD_LAYOUT R_FLOAT32   FNC_VALUES 1 FLOAT32_IEEE ROW_DIR DIRECT /end RECORD_LAYOUT
    /begin RECORD_LAYOUT R_FLOAT64   FNC_VALUES 1 FLOAT64_IEEE ROW_DIR DIRECT /end RECORD_LAYOUT

    /begin COMPU_METHOD CM_IDENTICAL "Direct 1:1 mapping" IDENTICAL "%6.3" "" /end COMPU_METHOD
    /begin COMPU_METHOD CM_VELOCITY_MPS "Scaled velocity" LINEAR "%6.2" "m/s"
      COEFFS_LINEAR 0.1 0.0
    /end COMPU_METHOD
    /begin COMPU_METHOD CM_GAIN_RAT "Rational gain" RAT_FUNC "%6.3" "ratio"
      COEFFS 0.0 2.0 0.0 0.0 0.0 1.0
    /end COMPU_METHOD
    /begin COMPU_METHOD CM_ADAS_MODE "ADAS Operating Mode Enum" TAB_VERB "%s" ""
      COMPU_TAB_REF VT_ADAS_MODE
    /end COMPU_METHOD

    /begin COMPU_VTAB VT_ADAS_MODE "Operating Mode Table" TAB_VERB 4
      0 "OFF"
      1 "STANDBY"
      2 "ACTIVE_LKA"
      3 "ACTIVE_ACC"
    /end COMPU_VTAB

    /* Typedef leafs and nested structures for Perception Radar */
    /begin TYPEDEF_MEASUREMENT TM_FloatDist "Target distance" FLOAT32_IEEE CM_IDENTICAL 0 0 -500.0 500.0 PHYS_UNIT "m" /end TYPEDEF_MEASUREMENT
    /begin TYPEDEF_MEASUREMENT TM_FloatVel  "Target relative velocity" FLOAT32_IEEE CM_IDENTICAL 0 0 -100.0 100.0 PHYS_UNIT "m/s" /end TYPEDEF_MEASUREMENT
    /begin TYPEDEF_MEASUREMENT TM_U32Count  "Frame counter" ULONG CM_IDENTICAL 0 0 0 4294967295 PHYS_UNIT "frames" /end TYPEDEF_MEASUREMENT
    /begin TYPEDEF_CHARACTERISTIC TC_ConfThresh "Detection threshold" VALUE R_FLOAT32 0 CM_IDENTICAL 0.0 1.0 PHYS_UNIT "prob" /end TYPEDEF_CHARACTERISTIC

    /begin TYPEDEF_STRUCTURE TS_RadarTarget "Single tracked radar object" 0x10
      /begin STRUCTURE_COMPONENT distance_m TM_FloatDist 0x00 /end STRUCTURE_COMPONENT
      /begin STRUCTURE_COMPONENT velocity_mps TM_FloatVel 0x04 /end STRUCTURE_COMPONENT
      /begin STRUCTURE_COMPONENT lat_offset_m TM_FloatDist 0x08 /end STRUCTURE_COMPONENT
      /begin STRUCTURE_COMPONENT snr_db TM_FloatVel 0x0C /end STRUCTURE_COMPONENT
    /end TYPEDEF_STRUCTURE

    /begin TYPEDEF_STRUCTURE TS_RadarCluster "Front Radar Sensor Cluster" 0x30
      /begin STRUCTURE_COMPONENT frame_counter TM_U32Count 0x00 /end STRUCTURE_COMPONENT
      /begin STRUCTURE_COMPONENT min_confidence TC_ConfThresh 0x04 /end STRUCTURE_COMPONENT
      /begin STRUCTURE_COMPONENT targets TS_RadarTarget 0x08 MATRIX_DIM 2 /end STRUCTURE_COMPONENT
    /end TYPEDEF_STRUCTURE

    /begin INSTANCE Perception.RadarFront "Front Radar Cluster Instance" TS_RadarCluster 0x1000
      ECU_ADDRESS_EXTENSION 0
      /begin IF_DATA XCP
        /begin DAQ_EVENT FIXED_EVENT_LIST EVENT 0x0000 /end DAQ_EVENT
      /end IF_DATA
    /end INSTANCE

    /* Direct dot-separated and array measurements */
    /begin MEASUREMENT EgoMotion.Kinematics.velocity_x "Longitudinal ego velocity"
      FLOAT32_IEEE CM_IDENTICAL 0 0 -50.0 120.0
      ECU_ADDRESS 0x2000
      ECU_ADDRESS_EXTENSION 0
      PHYS_UNIT "m/s"
      /begin IF_DATA XCP /begin DAQ_EVENT FIXED_EVENT_LIST EVENT 0x0000 /end DAQ_EVENT /end IF_DATA
    /end MEASUREMENT

    /begin MEASUREMENT EgoMotion.Kinematics.yaw_rate_deg_s "Ego yaw rate"
      FLOAT64_IEEE CM_IDENTICAL 0 0 -180.0 180.0
      ECU_ADDRESS 0x2008
      ECU_ADDRESS_EXTENSION 0
      PHYS_UNIT "deg/s"
      /begin IF_DATA XCP /begin DAQ_EVENT FIXED_EVENT_LIST EVENT 0x0000 /end DAQ_EVENT /end IF_DATA
    /end MEASUREMENT

    /begin MEASUREMENT EgoMotion.Pose.covariance_diag "Pose 3-element diagonal covariance"
      FLOAT32_IEEE CM_IDENTICAL 0 0 0.0 1000.0
      ECU_ADDRESS 0x2010
      ECU_ADDRESS_EXTENSION 0
      MATRIX_DIM 3
      PHYS_UNIT "m^2"
      /begin IF_DATA XCP /begin DAQ_EVENT FIXED_EVENT_LIST EVENT 0x0001 /end DAQ_EVENT /end IF_DATA
    /end MEASUREMENT

    /begin MEASUREMENT SystemHealth.QnxCpuLoadPercent "QNX 7.1 SoC Core Load"
      UBYTE CM_IDENTICAL 0 0 0 100
      ECU_ADDRESS 0x2020
      ECU_ADDRESS_EXTENSION 0
      PHYS_UNIT "%"
      /begin IF_DATA XCP /begin DAQ_EVENT FIXED_EVENT_LIST EVENT 0x0001 /end DAQ_EVENT /end IF_DATA
    /end MEASUREMENT

    /* Calibration Parameters (CHARACTERISTIC) */
    /begin CHARACTERISTIC Cal.Controller.kp_gain "PID Proportional Gain"
      VALUE 0x3000 R_FLOAT32 0 CM_IDENTICAL 0.0 50.0
      ECU_ADDRESS_EXTENSION 0
      PHYS_UNIT "1/s"
    /end CHARACTERISTIC

    /begin CHARACTERISTIC Cal.Controller.max_steering_deg "Maximum Steering Angle Limit"
      VALUE 0x3008 R_FLOAT64 0 CM_IDENTICAL 0.0 45.0
      ECU_ADDRESS_EXTENSION 0
      PHYS_UNIT "deg"
    /end CHARACTERISTIC

    /begin CHARACTERISTIC Cal.Controller.max_tracked_objects "Maximum Tracked Objects"
      VALUE 0x3010 R_UWORD 0 CM_IDENTICAL 1 128
      ECU_ADDRESS_EXTENSION 0
      PHYS_UNIT "count"
    /end CHARACTERISTIC

    /begin CHARACTERISTIC Cal.Controller.enable_aeb "Enable Autonomous Emergency Braking"
      VALUE 0x3012 R_UBYTE 0 CM_IDENTICAL 0 1
      ECU_ADDRESS_EXTENSION 0
      PHYS_UNIT "bool"
    /end CHARACTERISTIC

    /begin CHARACTERISTIC Cal.Controller.operating_mode "ADAS Operating Mode"
      VALUE 0x3014 R_UBYTE 0 CM_ADAS_MODE 0 3
      ECU_ADDRESS_EXTENSION 0
    /end CHARACTERISTIC

    /begin CHARACTERISTIC Cal.Controller.speed_lookup_curve "4-Point Speed Gain Curve"
      VAL_BLK 0x3020 R_FLOAT32 0 CM_IDENTICAL 0.0 10.0
      ECU_ADDRESS_EXTENSION 0
      MATRIX_DIM 4
      PHYS_UNIT "gain"
    /end CHARACTERISTIC

  /end MODULE
/end PROJECT
"#
}
