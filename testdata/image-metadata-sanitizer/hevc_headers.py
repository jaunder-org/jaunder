#!/usr/bin/env python3
"""Syntax-only HEVC header reader for the owned still-IDR proof lane.

Authored from ITU-T H.265 V9 (09/2023), 7.3.1/2/3/6 and E.2.1.
Single base layer, Main Still Picture, 8-bit 4:2:0, 64x64 coded canvas;
no opaque extensions, HRD, references, tiles, PCM, scaling lists or SEI.
Reading a slice header is NOT coded-data ownership validation.
"""


class HEVCDomainError(ValueError):
    """Explicit malformed or outside-proved-envelope HEVC result."""


def require(condition, detail):
    if not condition:
        raise HEVCDomainError("invalid/unsupported HEVC " + detail)


def rbsp(nal, kind):
    require(2 < len(nal) <= 8 * 1024 * 1024, "NAL size")
    require(nal[0] == kind << 1 and nal[1] == 1, "NAL type/layer/temporal header")
    out, zeros, at = bytearray(), 0, 2
    while at < len(nal):
        value = nal[at]
        if zeros == 2:
            require(value >= 3, "missing emulation prevention")
            if value == 3:
                require(at + 1 < len(nal) and nal[at + 1] <= 3,
                        "emulation prevention suffix")
                zeros = 0
                at += 1
                continue
        out.append(value)
        zeros = zeros + 1 if value == 0 else 0
        at += 1
    require(out and out[-1] != 0, "NAL final zero byte")
    return bytes(out)


class Bits:
    def __init__(self, data):
        self.data, self.at, self.fields = data, 0, {}

    def u(self, count, name):
        require(0 <= count <= 64 and self.at + count <= len(self.data) * 8,
                "truncated " + name)
        start, value = self.at, 0
        for _ in range(count):
            value = (value << 1) | ((self.data[self.at >> 3] >> (7 - (self.at & 7))) & 1)
            self.at += 1
        require(len(self.fields) < 1024, "header record budget")
        self.fields[name] = {"bit": start, "width": count, "value": value}
        return value

    def ue(self, name, maximum=65535):
        start, zeros = self.at, 0
        while self.u(1, name + ".prefix") == 0:
            zeros += 1
            require(zeros <= 16, "Exp-Golomb budget " + name)
        value = (1 << zeros) - 1 + self.u(zeros, name + ".suffix")
        require(value <= maximum, "range " + name)
        self.fields[name] = {"bit": start, "width": self.at - start, "value": value}
        return value

    def se(self, name, minimum, maximum):
        value = self.ue(name, 2 * max(abs(minimum), abs(maximum)))
        value = (value + 1) // 2 if value & 1 else -(value // 2)
        require(minimum <= value <= maximum, "range " + name)
        self.fields[name]["value"] = value
        return value

    def equal(self, count, value, name):
        require(self.u(count, name) == value, name)

    def zero(self, name):
        self.equal(1, 0, name)

    def align(self, name):
        self.equal(1, 1, name + ".one")
        if self.at & 7:
            self.equal(8 - (self.at & 7), 0, name + ".zero")

    def finish(self):
        self.align("rbsp_trailing_bits")
        require(self.at == len(self.data) * 8, "parameter trailing bytes")


def ptl(bits):
    bits.equal(2, 0, "profile_space")
    bits.zero("tier_flag")
    bits.equal(5, 3, "Main Still Picture profile_idc")
    compat = bits.u(32, "compatibility")
    require(compat == 0x70000000, "profile compatibility")
    bits.equal(1, 1, "progressive_source_flag")
    bits.zero("interlaced_source_flag")
    bits.u(1, "non_packed_constraint_flag")
    bits.equal(1, 1, "frame_only_constraint_flag")
    if compat & (1 << 29):
        bits.equal(7, 0, "reserved_zero_7bits")
        bits.u(1, "one_picture_only_constraint_flag")
        bits.equal(35, 0, "reserved_zero_35bits")
    else:
        bits.equal(43, 0, "reserved_zero_43bits")
    bits.zero("inbld_flag")
    # The frozen hvcC declares level30; no parameter may require more.
    bits.equal(8, 30, "level_idc")


def ordering(bits, prefix):
    bits.u(1, prefix + "_ordering_info_present_flag")
    bits.ue(prefix + "_max_dec_pic_buffering_minus1", 15)
    require(bits.ue(prefix + "_max_num_reorder_pics", 15) == 0, "still reordering")
    bits.ue(prefix + "_max_latency_increase_plus1")


def vps(nal):
    bits = Bits(rbsp(nal, 32))
    identity = bits.u(4, "vps_id")
    bits.equal(1, 1, "base_layer_internal_flag")
    bits.equal(1, 1, "base_layer_available_flag")
    bits.equal(6, 0, "max_layers_minus1")
    bits.equal(3, 0, "max_sub_layers_minus1")
    bits.equal(1, 1, "temporal_id_nesting_flag")
    bits.equal(16, 65535, "reserved_ffff")
    ptl(bits)
    ordering(bits, "vps")
    bits.equal(6, 0, "max_layer_id")
    require(bits.ue("num_layer_sets_minus1") == 0, "layer sets")
    bits.zero("vps_timing_info_present_flag")
    bits.zero("vps_extension_flag")
    bits.finish()
    return {"id": identity, "fields": bits.fields}


def vui(bits):
    if bits.u(1, "aspect_ratio_info_present_flag"):
        require(bits.u(8, "aspect_ratio_idc") == 1, "non-square SAR")
    bits.zero("overscan_info_present_flag")
    if bits.u(1, "video_signal_type_present_flag"):
        require(bits.u(3, "video_format") == 5, "video format")
        bits.equal(1, 1, "video_full_range_flag")
        bits.equal(1, 1, "colour_description_present_flag")
        bits.equal(8, 1, "colour_primaries")
        bits.equal(8, 13, "transfer_characteristics")
        bits.equal(8, 6, "matrix_coeffs")
    if bits.u(1, "chroma_loc_info_present_flag"):
        bits.ue("chroma_sample_loc_type_top_field", 5)
        bits.ue("chroma_sample_loc_type_bottom_field", 5)
    bits.zero("neutral_chroma_indication_flag")
    bits.zero("field_seq_flag")
    bits.zero("frame_field_info_present_flag")
    bits.zero("default_display_window_flag")
    if bits.u(1, "vui_timing_info_present_flag"):
        require(bits.u(32, "vui_num_units_in_tick") > 0, "VUI timing tick")
        require(bits.u(32, "vui_time_scale") > 0, "VUI time scale")
        if bits.u(1, "vui_poc_proportional_to_timing_flag"):
            bits.ue("vui_num_ticks_poc_diff_one_minus1")
        bits.zero("vui_hrd_parameters_present_flag")
    if bits.u(1, "bitstream_restriction_flag"):
        bits.u(1, "tiles_fixed_structure_flag")
        bits.u(1, "motion_vectors_over_pic_boundaries_flag")
        bits.u(1, "restricted_ref_pic_lists_flag")
        bits.ue("min_spatial_segmentation_idc", 4095)
        bits.ue("max_bytes_per_pic_denom", 16)
        bits.ue("max_bits_per_min_cu_denom", 16)
        bits.ue("log2_max_mv_length_horizontal", 15)
        bits.ue("log2_max_mv_length_vertical", 15)


def sps(nal, video):
    bits = Bits(rbsp(nal, 33))
    bits.equal(4, video["id"], "sps_vps_id")
    bits.equal(3, 0, "sps_max_sub_layers_minus1")
    bits.equal(1, 1, "sps_temporal_id_nesting_flag")
    ptl(bits)
    identity = bits.ue("sps_id", 15)
    require(bits.ue("chroma_format_idc", 3) == 1, "8-bit 4:2:0 chroma")
    require(bits.ue("pic_width_in_luma_samples") == 64
            and bits.ue("pic_height_in_luma_samples") == 64, "64x64 coded canvas")
    bits.zero("conformance_window_flag")
    require(bits.ue("bit_depth_luma_minus8", 8) == 0
            and bits.ue("bit_depth_chroma_minus8", 8) == 0, "8-bit precision")
    bits.ue("log2_max_pic_order_cnt_lsb_minus4", 12)
    ordering(bits, "sps")
    min_cb = bits.ue("log2_min_luma_coding_block_size_minus3", 3) + 3
    ctb = min_cb + bits.ue("log2_diff_max_min_luma_coding_block_size", 3)
    min_tb = bits.ue("log2_min_luma_transform_block_size_minus2", 3) + 2
    max_tb = min_tb + bits.ue("log2_diff_max_min_luma_transform_block_size", 3)
    require((ctb, min_cb, min_tb, max_tb) == (4, 3, 2, 4),
            "CTB16/minCB8/minTB4/maxTB16")
    inter_depth = bits.ue("max_transform_hierarchy_depth_inter", 4)
    intra_depth = bits.ue("max_transform_hierarchy_depth_intra", 4)
    require(inter_depth == 0, "inter transform depth0")
    require(intra_depth == 1, "intra transform depth1")
    bits.zero("scaling_list_enabled_flag")
    bits.u(1, "amp_enabled_flag")
    sao = bits.u(1, "sample_adaptive_offset_enabled_flag")
    require(sao == 1, "sample_adaptive_offset_enabled_flag")
    bits.zero("pcm_enabled_flag")
    require(bits.ue("num_short_term_ref_pic_sets", 64) == 0, "reference picture sets")
    bits.zero("long_term_ref_pics_present_flag")
    bits.u(1, "sps_temporal_mvp_enabled_flag")
    bits.u(1, "strong_intra_smoothing_enabled_flag")
    if bits.u(1, "vui_parameters_present_flag"):
        vui(bits)
    bits.zero("sps_extension_present_flag")
    bits.finish()
    return {"id": identity, "min_cb": min_cb, "ctb": ctb, "min_tb": min_tb,
            "max_tb": max_tb, "intra_depth": intra_depth, "sao": sao, "fields": bits.fields}


def pps(nal, sequence):
    bits = Bits(rbsp(nal, 34))
    identity = bits.ue("pps_id", 63)
    require(bits.ue("pps_sps_id", 15) == sequence["id"], "PPS SPS association")
    bits.zero("dependent_slice_segments_enabled_flag")
    output = bits.u(1, "output_flag_present_flag")
    require(output == 0, "output_flag_present_flag")
    bits.equal(3, 0, "num_extra_slice_header_bits")
    hiding = bits.u(1, "sign_data_hiding_enabled_flag")
    require(hiding == 1, "sign_data_hiding_enabled_flag")
    bits.u(1, "cabac_init_present_flag")
    bits.ue("num_ref_idx_l0_default_active_minus1", 14)
    bits.ue("num_ref_idx_l1_default_active_minus1", 14)
    qp = 26 + bits.se("init_qp_minus26", -26, 25)
    require(qp == 26, "PPS init QP26")
    bits.zero("constrained_intra_pred_flag")
    skip = bits.u(1, "transform_skip_enabled_flag")
    require(skip == 0, "transform_skip_enabled_flag")
    delta = bits.u(1, "cu_qp_delta_enabled_flag")
    require(delta == 1, "cu_qp_delta_enabled_flag")
    delta_depth = bits.ue("diff_cu_qp_delta_depth", 3)
    require(delta_depth == 0, "diff_cu_qp_delta_depth")
    bits.se("pps_cb_qp_offset", -12, 12)
    bits.se("pps_cr_qp_offset", -12, 12)
    chroma_offsets = bits.u(1, "pps_slice_chroma_qp_offsets_present_flag")
    require(chroma_offsets == 0, "pps_slice_chroma_qp_offsets_present_flag")
    bits.zero("weighted_pred_flag")
    bits.zero("weighted_bipred_flag")
    bits.zero("transquant_bypass_enabled_flag")
    bits.zero("tiles_enabled_flag")
    bits.zero("entropy_coding_sync_enabled_flag")
    loop = bits.u(1, "pps_loop_filter_across_slices_enabled_flag")
    override, disabled = 0, 0
    control = bits.u(1, "deblocking_filter_control_present_flag")
    require(control == 0, "deblocking_filter_control_present_flag")
    if control:
        override = bits.u(1, "deblocking_filter_override_enabled_flag")
        disabled = bits.u(1, "pps_deblocking_filter_disabled_flag")
        if not disabled:
            bits.se("pps_beta_offset_div2", -6, 6)
            bits.se("pps_tc_offset_div2", -6, 6)
    bits.zero("pps_scaling_list_data_present_flag")
    bits.u(1, "lists_modification_present_flag")
    bits.ue("log2_parallel_merge_level_minus2", 4)
    bits.zero("slice_segment_header_extension_present_flag")
    bits.zero("pps_extension_present_flag")
    bits.finish()
    return {"id": identity, "output": output, "hiding": hiding, "skip": skip,
            "qp": qp, "delta": delta, "delta_depth": delta_depth,
            "chroma_offsets": chroma_offsets, "loop": loop, "override": override,
            "disabled": disabled, "fields": bits.fields}


def slice_header(nal, sequence, picture):
    data = rbsp(nal, 20)
    bits = Bits(data)
    bits.equal(1, 1, "first_slice_segment_in_pic_flag")
    bits.u(1, "no_output_of_prior_pics_flag")
    require(bits.ue("slice_pic_parameter_set_id", 63) == picture["id"], "slice PPS association")
    require(bits.ue("slice_type", 2) == 2, "I slice")
    if picture["output"]:
        bits.equal(1, 1, "pic_output_flag")
    sao_y = bits.u(1, "slice_sao_luma_flag") if sequence["sao"] else 0
    sao_c = bits.u(1, "slice_sao_chroma_flag") if sequence["sao"] else 0
    require(sao_y == 1, "slice_sao_luma_flag")
    require(sao_c == 1, "slice_sao_chroma_flag")
    qp = picture["qp"] + bits.se("slice_qp_delta", -51, 51)
    require(qp == 22, "SliceQpY22")
    if picture["chroma_offsets"]:
        bits.se("slice_cb_qp_offset", -12, 12)
        bits.se("slice_cr_qp_offset", -12, 12)
    disabled = picture["disabled"]
    if picture["override"] and bits.u(1, "deblocking_filter_override_flag"):
        disabled = bits.u(1, "slice_deblocking_filter_disabled_flag")
        if not disabled:
            bits.se("slice_beta_offset_div2", -6, 6)
            bits.se("slice_tc_offset_div2", -6, 6)
    if picture["loop"] and (sao_y or sao_c or not disabled):
        bits.u(1, "slice_loop_filter_across_slices_enabled_flag")
    bits.align("byte_alignment")
    return {"data": data, "cabac_start": bits.at // 8, "qp": qp,
            "sao_y": sao_y, "sao_c": sao_c, "fields": bits.fields}
