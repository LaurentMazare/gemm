use dyn_stack::{DynStack, GlobalMemBuffer, StackReq};
use gemm_common::{
    cache::{div_ceil, kernel_params, KernelParams},
    gemm::{get_threading_threshold, par_for_each, CACHELINE_ALIGN, L2_SLAB},
    microkernel::MicroKernelFn,
    pack_operands::quick_zero,
    Parallelism, Ptr,
};
use half::slice::HalfFloatSliceExt;
type T = half::f16;

#[inline(always)]
unsafe fn pack_generic_inner_loop<const N: usize, const DST_WIDTH: usize>(
    mut dst: *mut f32,
    mut src: *const T,
    src_rs: isize,
    src_cs: isize,
    src_width: usize,
    k: usize,
) {
    if src_width == DST_WIDTH {
        if src_rs == 1 {
            for _ in 0..k {
                let val = (src as *const [T; DST_WIDTH]).read();
                val.convert_to_f32_slice(core::slice::from_raw_parts_mut(dst, DST_WIDTH));

                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        } else {
            for _ in 0..k {
                for j in 0..DST_WIDTH {
                    *dst.add(j) = (*src.offset(j as isize * src_rs)).into();
                }
                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        }
    } else if src_width == N {
        if src_rs == 1 {
            for _ in 0..k {
                let val = (src as *const [T; N]).read();
                val.convert_to_f32_slice(core::slice::from_raw_parts_mut(dst, N));

                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        } else {
            for _ in 0..k {
                for j in 0..N {
                    *dst.add(j) = (*src.offset(j as isize * src_rs)).into();
                }
                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        }
    } else if src_width == 2 * N {
        if src_rs == 1 {
            for _ in 0..k {
                let val0 = (src as *const [T; N]).read();
                let val1 = (src.add(N) as *const [T; N]).read();
                val0.convert_to_f32_slice(core::slice::from_raw_parts_mut(dst, N));
                val1.convert_to_f32_slice(core::slice::from_raw_parts_mut(dst.add(N), N));

                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        } else {
            for _ in 0..k {
                for j in 0..2 * N {
                    *dst.add(j) = (*src.offset(j as isize * src_rs)).into();
                }
                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        }
    } else {
        for _ in 0..k {
            for j in 0..src_width {
                *dst.add(j) = (*src.offset(j as isize * src_rs)).into();
            }
            quick_zero(core::slice::from_raw_parts_mut(
                dst.add(src_width),
                DST_WIDTH - src_width,
            ));
            src = src.wrapping_offset(src_cs);
            dst = dst.add(DST_WIDTH);
        }
    }
}

// DIRECT copy of [`pack_generic_inner_loop`]  but adapted for pure f16 inner
#[inline(always)]
unsafe fn pack_generic_inner_loop_f16<const N: usize, const DST_WIDTH: usize>(
    mut dst: *mut T,
    mut src: *const T,
    src_rs: isize,
    src_cs: isize,
    src_width: usize,
    k: usize,
) {
    if src_width == DST_WIDTH {
        if src_rs == 1 {
            for _ in 0..k {
                // let val = (src as *const [T; DST_WIDTH]).read();
                // val.convert_to_f32_slice(core::slice::from_raw_parts_mut(dst, DST_WIDTH));
                std::ptr::copy_nonoverlapping(src, dst, DST_WIDTH);

                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        } else {
            for _ in 0..k {
                for j in 0..DST_WIDTH {
                    *dst.add(j) = (*src.offset(j as isize * src_rs)).into();
                }
                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        }
    } else if src_width == N {
        if src_rs == 1 {
            for _ in 0..k {
                // let val = (src as *const [T; N]).read();
                // val.convert_to_f32_slice(core::slice::from_raw_parts_mut(dst, N));
                std::ptr::copy_nonoverlapping(src, dst, N);
                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        } else {
            for _ in 0..k {
                for j in 0..N {
                    *dst.add(j) = (*src.offset(j as isize * src_rs)).into();
                }
                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        }
    } else if src_width == 2 * N {
        if src_rs == 1 {
            for _ in 0..k {
                // let val0 = (src as *const [T; N]).read();
                // let val1 = (src.add(N) as *const [T; N]).read();
                // val0.convert_to_f32_slice(core::slice::from_raw_parts_mut(dst, N));
                // val1.convert_to_f32_slice(core::slice::from_raw_parts_mut(dst.add(N), N));
                std::ptr::copy_nonoverlapping(src, dst, 2 * N);

                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        } else {
            for _ in 0..k {
                for j in 0..2 * N {
                    *dst.add(j) = (*src.offset(j as isize * src_rs)).into();
                }
                src = src.wrapping_offset(src_cs);
                dst = dst.add(DST_WIDTH);
            }
        }
    } else {
        for _ in 0..k {
            for j in 0..src_width {
                *dst.add(j) = (*src.offset(j as isize * src_rs)).into();
            }
            quick_zero(core::slice::from_raw_parts_mut(
                dst.add(src_width),
                DST_WIDTH - src_width,
            ));
            src = src.wrapping_offset(src_cs);
            dst = dst.add(DST_WIDTH);
        }
    }
}

#[inline(always)]
unsafe fn pack_generic<const N: usize, const DST_WIDTH: usize>(
    m: usize,
    k: usize,
    mut dst: *mut f32,
    mut src: *const T,
    src_cs: isize,
    src_rs: isize,
    dst_stride: usize,
) {
    let m_width = m / DST_WIDTH * DST_WIDTH;

    let mut i = 0;
    while i < m_width {
        pack_generic_inner_loop::<N, DST_WIDTH>(dst, src, src_rs, src_cs, DST_WIDTH, k);
        src = src.wrapping_offset(src_rs * DST_WIDTH as isize);
        dst = dst.add(dst_stride);

        i += DST_WIDTH;
    }
    if i < m {
        pack_generic_inner_loop::<N, DST_WIDTH>(dst, src, src_rs, src_cs, m - i, k);
    }
}

// DIRECT copy of [`pack_generic`]  but adapted for pure f16
#[inline(always)]
unsafe fn pack_generic_f16<const N: usize, const DST_WIDTH: usize>(
    m: usize,
    k: usize,
    mut dst: *mut T,
    mut src: *const T,
    src_cs: isize,
    src_rs: isize,
    dst_stride: usize,
) {
    let m_width = m / DST_WIDTH * DST_WIDTH;

    let mut i = 0;
    while i < m_width {
        pack_generic_inner_loop_f16::<N, DST_WIDTH>(dst, src, src_rs, src_cs, DST_WIDTH, k);
        src = src.wrapping_offset(src_rs * DST_WIDTH as isize);
        dst = dst.add(dst_stride);

        i += DST_WIDTH;
    }
    if i < m {
        pack_generic_inner_loop_f16::<N, DST_WIDTH>(dst, src, src_rs, src_cs, m - i, k);
    }
}

#[inline(never)]
pub unsafe fn pack_lhs<const N: usize, const MR: usize>(
    m: usize,
    k: usize,
    dst: Ptr<f32>,
    src: Ptr<T>,
    src_cs: isize,
    src_rs: isize,
    dst_stride: usize,
) {
    let dst = dst.0;
    let src = src.0;
    pack_generic::<N, MR>(m, k, dst, src, src_cs, src_rs, dst_stride);
}

#[inline(never)]
pub unsafe fn pack_rhs<const N: usize, const NR: usize>(
    n: usize,
    k: usize,
    dst: Ptr<f32>,
    src: Ptr<T>,
    src_cs: isize,
    src_rs: isize,
    dst_stride: usize,
) {
    let dst = dst.0;
    let src = src.0;
    pack_generic::<N, NR>(n, k, dst, src, src_rs, src_cs, dst_stride);
}

// DIRECT copy of [`pack_lhs`]  but adapted for pure f16
#[inline(never)]
pub unsafe fn pack_lhs_f16<const N: usize, const MR: usize>(
    m: usize,
    k: usize,
    dst: Ptr<T>,
    src: Ptr<T>,
    src_cs: isize,
    src_rs: isize,
    dst_stride: usize,
) {
    let dst = dst.0;
    let src = src.0;
    pack_generic_f16::<N, MR>(m, k, dst, src, src_cs, src_rs, dst_stride);
}

// DIRECT copy of [`pack_rhs`]  but adapted for pure f16
#[inline(never)]
pub unsafe fn pack_rhs_f16<const N: usize, const NR: usize>(
    n: usize,
    k: usize,
    dst: Ptr<T>,
    src: Ptr<T>,
    src_cs: isize,
    src_rs: isize,
    dst_stride: usize,
) {
    let dst = dst.0;
    let src = src.0;
    pack_generic_f16::<N, NR>(n, k, dst, src, src_rs, src_cs, dst_stride);
}

#[inline(always)]
pub unsafe fn gemm_basic_generic<
    const N: usize,
    const MR: usize,
    const NR: usize,
    const MR_DIV_N: usize,
>(
    m: usize,
    n: usize,
    k: usize,
    dst: *mut T,
    dst_cs: isize,
    dst_rs: isize,
    read_dst: bool,
    lhs: *const T,
    lhs_cs: isize,
    lhs_rs: isize,
    rhs: *const T,
    rhs_cs: isize,
    rhs_rs: isize,
    mut alpha: T,
    beta: T,
    dispatcher: &[[MicroKernelFn<f32>; NR]; MR_DIV_N],
    parallelism: Parallelism,
) {
    if m == 0 || n == 0 {
        return;
    }
    if !read_dst {
        alpha = T::ZERO;
    }

    if k == 0 {
        if alpha == T::ZERO {
            for j in 0..n {
                for i in 0..m {
                    *dst.offset(i as isize * dst_rs + j as isize * dst_cs) = T::ZERO;
                }
            }
            return;
        }
        if alpha == T::ONE {
            return;
        }

        for j in 0..n {
            for i in 0..m {
                let dst = dst.offset(i as isize * dst_rs + j as isize * dst_cs);
                *dst = alpha * *dst;
            }
        }
        return;
    }

    let KernelParams { kc, mc, nc } = kernel_params(m, n, k, MR, NR, core::mem::size_of::<f32>());
    let nc = if nc > 0 {
        nc
    } else {
        match parallelism {
            Parallelism::None => 128 * NR,
            Parallelism::Rayon(_) => div_ceil(n, NR) * NR,
        }
    };

    let simd_align = CACHELINE_ALIGN;

    let packed_rhs_stride = kc * NR;
    let packed_lhs_stride = kc * MR;

    let dst = Ptr(dst);
    let lhs = Ptr(lhs as *mut T);
    let rhs = Ptr(rhs as *mut T);

    let mut mem = GlobalMemBuffer::new(StackReq::new_aligned::<f32>(
        packed_rhs_stride * (nc / NR),
        simd_align,
    ));

    let stack = DynStack::new(&mut mem);
    let mut packed_rhs_storage = stack
        .make_aligned_uninit::<f32>(packed_rhs_stride * (nc / NR), simd_align)
        .0;

    let packed_rhs = Ptr(packed_rhs_storage.as_mut_ptr() as *mut f32);

    let packed_rhs_rs = NR as isize;
    let packed_rhs_cs = 1;

    let mut col_outer = 0;
    while col_outer != n {
        let n_chunk = nc.min(n - col_outer);

        let mut alpha = alpha.to_f32();

        let mut depth_outer = 0;
        while depth_outer != k {
            let k_chunk = kc.min(k - depth_outer);
            let alpha_status = if alpha == 0.0 {
                0
            } else if alpha == 1.0 {
                1
            } else {
                2
            };

            let n_threads = match parallelism {
                Parallelism::None => 1,
                Parallelism::Rayon(max_threads) => {
                    let threading_threshold = get_threading_threshold();
                    let max_threads = if max_threads == 0 {
                        rayon::current_num_threads()
                    } else {
                        max_threads
                    };
                    let total_work = m * n_chunk * k_chunk;
                    let n_threads = if total_work > threading_threshold {
                        std::cmp::max(
                            1,
                            std::cmp::min(
                                max_threads,
                                (total_work - threading_threshold + 1) / threading_threshold,
                            ),
                        )
                    } else {
                        1
                    };
                    n_threads
                }
            };

            // pack rhs
            if n_threads <= 1 {
                pack_rhs::<1, NR>(
                    n_chunk,
                    k_chunk,
                    packed_rhs,
                    rhs.wrapping_offset(
                        depth_outer as isize * rhs_rs + col_outer as isize * rhs_cs,
                    ),
                    rhs_cs,
                    rhs_rs,
                    packed_rhs_stride,
                );
            } else {
                let n_tasks = div_ceil(n_chunk, NR);
                let base = n_tasks / n_threads;
                let rem = n_tasks % n_threads;

                let tid_to_col_inner = |tid: usize| {
                    if tid == n_threads {
                        return n_chunk;
                    }

                    let col = if tid < rem {
                        NR * tid * (base + 1)
                    } else {
                        NR * (rem + tid * base)
                    };

                    col.min(n_chunk)
                };

                let func = |tid: usize| {
                    let col_inner = tid_to_col_inner(tid);
                    let ncols = tid_to_col_inner(tid + 1) - col_inner;
                    let j = col_inner / NR;

                    if ncols > 0 {
                        pack_rhs::<1, NR>(
                            ncols,
                            k_chunk,
                            packed_rhs.wrapping_add(j * packed_rhs_stride),
                            rhs.wrapping_offset(
                                depth_outer as isize * rhs_rs
                                    + (col_outer + col_inner) as isize * rhs_cs,
                            ),
                            rhs_cs,
                            rhs_rs,
                            packed_rhs_stride,
                        );
                    }
                };
                par_for_each(n_threads, func);
            }

            let n_col_mini_chunks = (n_chunk + (NR - 1)) / NR;

            let mut n_jobs = 0;
            let mut row_outer = 0;
            while row_outer != m {
                let mut m_chunk = mc.min(m - row_outer);
                if m_chunk > N {
                    m_chunk = m_chunk / N * N;
                }
                let n_row_mini_chunks = (m_chunk + (MR - 1)) / MR;
                n_jobs += n_col_mini_chunks * n_row_mini_chunks;
                row_outer += m_chunk;
            }

            // use a single thread for small workloads

            let func = move |tid| {
                L2_SLAB.with(|mem| {
                    let mut mem = mem.borrow_mut();
                    let stack = DynStack::new(&mut **mem);

                    let (mut packed_lhs_storage, _) =
                        stack.make_aligned_uninit::<f32>(packed_lhs_stride * (mc / MR), simd_align);

                    let packed_lhs = Ptr(packed_lhs_storage.as_mut_ptr() as *mut f32);

                    let min_jobs_per_thread = n_jobs / n_threads;
                    let rem = n_jobs - n_threads * min_jobs_per_thread;

                    // thread `tid` takes min_jobs_per_thread or min_jobs_per_thread + 1
                    let (job_start, job_end) = if tid < rem {
                        let start = tid * (min_jobs_per_thread + 1);
                        (start, start + min_jobs_per_thread + 1)
                    } else {
                        // start = rem * (min_jobs_per_thread + 1) + (tid - rem) * min_jobs_per_thread;
                        let start = tid * min_jobs_per_thread + rem;
                        (start, start + min_jobs_per_thread)
                    };

                    let mut row_outer = 0;
                    let mut job_id = 0;
                    while row_outer != m {
                        let mut m_chunk = mc.min(m - row_outer);
                        if m_chunk > N {
                            m_chunk = m_chunk / N * N;
                        }
                        let n_row_mini_chunks = (m_chunk + (MR - 1)) / MR;

                        let n_mini_jobs = n_col_mini_chunks * n_row_mini_chunks;

                        if job_id >= job_end {
                            return;
                        }
                        if job_id + n_mini_jobs < job_start {
                            row_outer += m_chunk;
                            job_id += n_mini_jobs;
                            continue;
                        }

                        let packed_lhs_cs = MR as isize;

                        pack_lhs::<N, MR>(
                            m_chunk,
                            k_chunk,
                            packed_lhs,
                            lhs.wrapping_offset(
                                row_outer as isize * lhs_rs + depth_outer as isize * lhs_cs,
                            ),
                            lhs_cs,
                            lhs_rs,
                            packed_lhs_stride,
                        );

                        let mut j = 0;
                        while j < n_col_mini_chunks {
                            let mut i = 0;
                            while i < n_row_mini_chunks {
                                let col_inner = NR * j;
                                let n_chunk_inner = NR.min(n_chunk - col_inner);

                                let row_inner = MR * i;
                                let m_chunk_inner = MR.min(m_chunk - row_inner);

                                let inner_idx = &mut i;
                                if job_id < job_start || job_id >= job_end {
                                    job_id += 1;
                                    *inner_idx += 1;
                                    continue;
                                }
                                job_id += 1;

                                let dst = dst.wrapping_offset(
                                    (row_outer + row_inner) as isize * dst_rs
                                        + (col_outer + col_inner) as isize * dst_cs,
                                );

                                let func = dispatcher[(m_chunk_inner + (N - 1)) / N - 1]
                                    [n_chunk_inner - 1];

                                let mut tmp = [[0.0f32; MR]; NR];

                                func(
                                    m_chunk_inner,
                                    n_chunk_inner,
                                    k_chunk,
                                    tmp.as_mut_ptr() as *mut f32,
                                    packed_lhs.wrapping_add(i * packed_lhs_stride).0,
                                    packed_rhs.wrapping_add(j * packed_rhs_stride).0,
                                    MR as isize,
                                    1,
                                    packed_lhs_cs,
                                    packed_rhs_rs,
                                    packed_rhs_cs,
                                    0.0,
                                    beta.into(),
                                    0,
                                    false,
                                    false,
                                    false,
                                    packed_lhs.wrapping_add((i + 1) * packed_lhs_stride).0,
                                );

                                match alpha_status {
                                    0 => {
                                        for j in 0..n_chunk_inner {
                                            for i in 0..m_chunk_inner {
                                                let dst = dst
                                                    .wrapping_offset(j as isize * dst_cs)
                                                    .wrapping_offset(i as isize * dst_rs)
                                                    .0;
                                                *dst = T::from_f32(tmp[j][i]);
                                            }
                                        }
                                    }
                                    1 => {
                                        for j in 0..n_chunk_inner {
                                            for i in 0..m_chunk_inner {
                                                let dst = dst
                                                    .wrapping_offset(j as isize * dst_cs)
                                                    .wrapping_offset(i as isize * dst_rs)
                                                    .0;
                                                *dst = T::from_f32((*dst).to_f32() + tmp[j][i]);
                                            }
                                        }
                                    }
                                    _ => {
                                        for j in 0..n_chunk_inner {
                                            for i in 0..m_chunk_inner {
                                                let dst = dst
                                                    .wrapping_offset(j as isize * dst_cs)
                                                    .wrapping_offset(i as isize * dst_rs)
                                                    .0;
                                                *dst = T::from_f32(
                                                    alpha * (*dst).to_f32() + tmp[j][i],
                                                );
                                            }
                                        }
                                    }
                                }

                                i += 1;
                            }
                            j += 1;
                        }

                        row_outer += m_chunk;
                    }
                });
            };

            match parallelism {
                Parallelism::None => func(0),
                Parallelism::Rayon(_) => {
                    if n_threads == 1 {
                        func(0);
                    } else {
                        par_for_each(n_threads, func);
                    }
                }
            }

            alpha = 1.0;
            depth_outer += k_chunk;
        }
        col_outer += n_chunk;
    }
}

// DIRECT copy of [`gemm_basic`]  but adapted for pure f16
#[inline(always)]
pub unsafe fn gemm_basic_f16<
    const N: usize,
    const MR: usize,
    const NR: usize,
    const MR_DIV_N: usize,
>(
    m: usize,
    n: usize,
    k: usize,
    dst: *mut T,
    dst_cs: isize,
    dst_rs: isize,
    read_dst: bool,
    lhs: *const T,
    lhs_cs: isize,
    lhs_rs: isize,
    rhs: *const T,
    rhs_cs: isize,
    rhs_rs: isize,
    mut alpha: T,
    beta: T,
    dispatcher: &[[MicroKernelFn<T>; NR]; MR_DIV_N],
    parallelism: Parallelism,
) {
    // println!("-- {m} {n} {k} \n lhs: {:?}\n  {:?}", std::slice::from_raw_parts(lhs, m * k), std::slice::from_raw_parts(rhs, n * k));
    if m == 0 || n == 0 {
        return;
    }
    if !read_dst {
        alpha = T::ZERO;
    }

    if k == 0 {
        if alpha == T::ZERO {
            for j in 0..n {
                for i in 0..m {
                    *dst.offset(i as isize * dst_rs + j as isize * dst_cs) = T::ZERO;
                }
            }
            return;
        }
        if alpha == T::ONE {
            return;
        }

        for j in 0..n {
            for i in 0..m {
                let dst = dst.offset(i as isize * dst_rs + j as isize * dst_cs);
                *dst = alpha * *dst;
            }
        }
        return;
    }

    let KernelParams { kc, mc, nc } = kernel_params(m, n, k, MR, NR, core::mem::size_of::<T>());
    let nc = if nc > 0 {
        nc
    } else {
        match parallelism {
            Parallelism::None => 128 * NR,
            Parallelism::Rayon(_) => div_ceil(n, NR) * NR,
        }
    };

    let simd_align = CACHELINE_ALIGN;

    let packed_rhs_stride = kc * NR;
    let packed_lhs_stride = kc * MR;

    let dst = Ptr(dst);
    let lhs = Ptr(lhs as *mut T);
    let rhs = Ptr(rhs as *mut T);

    let mut mem = GlobalMemBuffer::new(StackReq::new_aligned::<T>(
        packed_rhs_stride * (nc / NR),
        simd_align,
    ));

    let stack = DynStack::new(&mut mem);
    let mut packed_rhs_storage = stack
        .make_aligned_uninit::<T>(packed_rhs_stride * (nc / NR), simd_align)
        .0;

    let packed_rhs = Ptr(packed_rhs_storage.as_mut_ptr() as *mut T);

    let packed_rhs_rs = NR as isize;
    let packed_rhs_cs = 1;

    let mut col_outer = 0;
    while col_outer != n {
        let n_chunk = nc.min(n - col_outer);

        let mut alpha = alpha;

        let mut depth_outer = 0;
        while depth_outer != k {
            let k_chunk = kc.min(k - depth_outer);
            let alpha_status = if alpha == T::ZERO {
                0
            } else if alpha == T::ONE {
                1
            } else {
                2
            };

            let n_threads = match parallelism {
                Parallelism::None => 1,
                Parallelism::Rayon(max_threads) => {
                    let threading_threshold = get_threading_threshold();
                    let max_threads = if max_threads == 0 {
                        rayon::current_num_threads()
                    } else {
                        max_threads
                    };
                    let total_work = m * n_chunk * k_chunk;
                    let n_threads = if total_work > threading_threshold {
                        std::cmp::max(
                            1,
                            std::cmp::min(
                                max_threads,
                                (total_work - threading_threshold + 1) / threading_threshold,
                            ),
                        )
                    } else {
                        1
                    };
                    n_threads
                }
            };

            // pack rhs
            if n_threads <= 1 {
                pack_rhs_f16::<1, NR>(
                    n_chunk,
                    k_chunk,
                    packed_rhs,
                    rhs.wrapping_offset(
                        depth_outer as isize * rhs_rs + col_outer as isize * rhs_cs,
                    ),
                    rhs_cs,
                    rhs_rs,
                    packed_rhs_stride,
                );
            } else {
                let n_tasks = div_ceil(n_chunk, NR);
                let base = n_tasks / n_threads;
                let rem = n_tasks % n_threads;

                let tid_to_col_inner = |tid: usize| {
                    if tid == n_threads {
                        return n_chunk;
                    }

                    let col = if tid < rem {
                        NR * tid * (base + 1)
                    } else {
                        NR * (rem + tid * base)
                    };

                    col.min(n_chunk)
                };

                let func = |tid: usize| {
                    let col_inner = tid_to_col_inner(tid);
                    let ncols = tid_to_col_inner(tid + 1) - col_inner;
                    let j = col_inner / NR;

                    if ncols > 0 {
                        pack_rhs_f16::<1, NR>(
                            ncols,
                            k_chunk,
                            packed_rhs.wrapping_add(j * packed_rhs_stride),
                            rhs.wrapping_offset(
                                depth_outer as isize * rhs_rs
                                    + (col_outer + col_inner) as isize * rhs_cs,
                            ),
                            rhs_cs,
                            rhs_rs,
                            packed_rhs_stride,
                        );
                    }
                };
                par_for_each(n_threads, func);
            }

            let n_col_mini_chunks = (n_chunk + (NR - 1)) / NR;

            let mut n_jobs = 0;
            let mut row_outer = 0;
            while row_outer != m {
                let mut m_chunk = mc.min(m - row_outer);
                if m_chunk > N {
                    m_chunk = m_chunk / N * N;
                }
                let n_row_mini_chunks = (m_chunk + (MR - 1)) / MR;
                n_jobs += n_col_mini_chunks * n_row_mini_chunks;
                row_outer += m_chunk;
            }

            // use a single thread for small workloads

            let func = move |tid| {
                L2_SLAB.with(|mem| {
                    let mut mem = mem.borrow_mut();
                    let stack = DynStack::new(&mut **mem);

                    let (mut packed_lhs_storage, _) =
                        stack.make_aligned_uninit::<T>(packed_lhs_stride * (mc / MR), simd_align);

                    let packed_lhs = Ptr(packed_lhs_storage.as_mut_ptr() as *mut T);

                    let min_jobs_per_thread = n_jobs / n_threads;
                    let rem = n_jobs - n_threads * min_jobs_per_thread;

                    // thread `tid` takes min_jobs_per_thread or min_jobs_per_thread + 1
                    let (job_start, job_end) = if tid < rem {
                        let start = tid * (min_jobs_per_thread + 1);
                        (start, start + min_jobs_per_thread + 1)
                    } else {
                        // start = rem * (min_jobs_per_thread + 1) + (tid - rem) * min_jobs_per_thread;
                        let start = tid * min_jobs_per_thread + rem;
                        (start, start + min_jobs_per_thread)
                    };

                    let mut row_outer = 0;
                    let mut job_id = 0;
                    while row_outer != m {
                        let mut m_chunk = mc.min(m - row_outer);
                        if m_chunk > N {
                            m_chunk = m_chunk / N * N;
                        }
                        let n_row_mini_chunks = (m_chunk + (MR - 1)) / MR;

                        let n_mini_jobs = n_col_mini_chunks * n_row_mini_chunks;

                        if job_id >= job_end {
                            return;
                        }
                        if job_id + n_mini_jobs < job_start {
                            row_outer += m_chunk;
                            job_id += n_mini_jobs;
                            continue;
                        }

                        let packed_lhs_cs = MR as isize;

                        pack_lhs_f16::<N, MR>(
                            m_chunk,
                            k_chunk,
                            packed_lhs,
                            lhs.wrapping_offset(
                                row_outer as isize * lhs_rs + depth_outer as isize * lhs_cs,
                            ),
                            lhs_cs,
                            lhs_rs,
                            packed_lhs_stride,
                        );

                        let mut j = 0;
                        while j < n_col_mini_chunks {
                            let mut i = 0;
                            while i < n_row_mini_chunks {
                                let col_inner = NR * j;
                                let n_chunk_inner = NR.min(n_chunk - col_inner);

                                let row_inner = MR * i;
                                let m_chunk_inner = MR.min(m_chunk - row_inner);

                                let inner_idx = &mut i;
                                if job_id < job_start || job_id >= job_end {
                                    job_id += 1;
                                    *inner_idx += 1;
                                    continue;
                                }
                                job_id += 1;

                                let dst = dst.wrapping_offset(
                                    (row_outer + row_inner) as isize * dst_rs
                                        + (col_outer + col_inner) as isize * dst_cs,
                                );

                                let func = dispatcher[(m_chunk_inner + (N - 1)) / N - 1]
                                    [n_chunk_inner - 1];

                                let mut tmp = [[T::ZERO; MR]; NR];

                                func(
                                    m_chunk_inner,
                                    n_chunk_inner,
                                    k_chunk,
                                    tmp.as_mut_ptr() as *mut T,
                                    packed_lhs.wrapping_add(i * packed_lhs_stride).0,
                                    packed_rhs.wrapping_add(j * packed_rhs_stride).0,
                                    MR as isize,
                                    1,
                                    packed_lhs_cs,
                                    packed_rhs_rs,
                                    packed_rhs_cs,
                                    T::ZERO,
                                    beta,
                                    0,
                                    false,
                                    false,
                                    false,
                                    packed_lhs.wrapping_add((i + 1) * packed_lhs_stride).0,
                                );

                                match alpha_status {
                                    0 => {
                                        for j in 0..n_chunk_inner {
                                            for i in 0..m_chunk_inner {
                                                let dst = dst
                                                    .wrapping_offset(j as isize * dst_cs)
                                                    .wrapping_offset(i as isize * dst_rs)
                                                    .0;
                                                *dst = tmp[j][i];
                                            }
                                        }
                                    }
                                    1 => {
                                        for j in 0..n_chunk_inner {
                                            for i in 0..m_chunk_inner {
                                                let dst = dst
                                                    .wrapping_offset(j as isize * dst_cs)
                                                    .wrapping_offset(i as isize * dst_rs)
                                                    .0;
                                                *dst = (*dst) + tmp[j][i];
                                            }
                                        }
                                    }
                                    _ => {
                                        for j in 0..n_chunk_inner {
                                            for i in 0..m_chunk_inner {
                                                let dst = dst
                                                    .wrapping_offset(j as isize * dst_cs)
                                                    .wrapping_offset(i as isize * dst_rs)
                                                    .0;
                                                *dst = alpha * (*dst) + tmp[j][i];
                                            }
                                        }
                                    }
                                }

                                i += 1;
                            }
                            j += 1;
                        }

                        row_outer += m_chunk;
                    }
                });
            };

            match parallelism {
                Parallelism::None => func(0),
                Parallelism::Rayon(_) => {
                    if n_threads == 1 {
                        func(0);
                    } else {
                        par_for_each(n_threads, func);
                    }
                }
            }

            alpha = T::ONE;
            depth_outer += k_chunk;
        }
        col_outer += n_chunk;
    }
}

pub mod f16 {
    use super::gemm_basic_generic;
    use gemm_common::Parallelism;

    type T = half::f16;
    type GemmTy = unsafe fn(
        usize,
        usize,
        usize,
        *mut T,
        isize,
        isize,
        bool,
        *const T,
        isize,
        isize,
        *const T,
        isize,
        isize,
        T,
        T,
        bool,
        bool,
        bool,
        Parallelism,
    );

    fn init_gemm_fn() -> GemmTy {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            #[cfg(feature = "nightly")]
            if gemm_common::feature_detected!("avx512f") {
                return avx512f::gemm_basic;
            }
            if gemm_common::feature_detected!("fma") {
                fma::gemm_basic
            } else if gemm_common::feature_detected!("avx") {
                avx::gemm_basic
            } else if gemm_common::feature_detected!("sse")
                && gemm_common::feature_detected!("sse2")
            {
                sse::gemm_basic
            } else {
                scalar::gemm_basic
            }
        }

        #[cfg(target_arch = "aarch64")]
        #[cfg(target_feature = "fp16")]
        {
            if gemm_common::feature_detected!("neon") {
                neon::gemm_basic
            } else {
                scalar::gemm_basic
            }
        }

        #[cfg(target_arch = "aarch64")]
        #[cfg(not(target_feature = "fp16"))]
        {
            scalar::gemm_basic
        }

        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
        {
            scalar::gemm_basic
        }
    }

    lazy_static::lazy_static! {
        pub static ref GEMM: GemmTy = init_gemm_fn();
    }

    mod scalar {
        use super::*;
        use gemm_f32::microkernel::scalar::f32::*;
        const N: usize = 1;

        #[inline(never)]
        pub unsafe fn gemm_basic(
            m: usize,
            n: usize,
            k: usize,
            dst: *mut T,
            dst_cs: isize,
            dst_rs: isize,
            read_dst: bool,
            lhs: *const T,
            lhs_cs: isize,
            lhs_rs: isize,
            rhs: *const T,
            rhs_cs: isize,
            rhs_rs: isize,
            alpha: T,
            beta: T,
            _conj_dst: bool,
            _conj_lhs: bool,
            _conj_rhs: bool,
            parallelism: gemm_common::Parallelism,
        ) {
            gemm_basic_generic::<N, { MR_DIV_N * N }, NR, MR_DIV_N>(
                m,
                n,
                k,
                dst,
                dst_cs,
                dst_rs,
                read_dst,
                lhs,
                lhs_cs,
                lhs_rs,
                rhs,
                rhs_cs,
                rhs_rs,
                alpha,
                beta,
                &UKR,
                parallelism,
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[cfg(target_feature = "fp16")]
    mod neon {
        use super::*;
        use crate::microkernel::neon::f16::{MR_DIV_N, NR, UKR};
        const N: usize = 8;

        #[inline(never)]
        pub unsafe fn gemm_basic(
            m: usize,
            n: usize,
            k: usize,
            dst: *mut T,
            dst_cs: isize,
            dst_rs: isize,
            read_dst: bool,
            lhs: *const T,
            lhs_cs: isize,
            lhs_rs: isize,
            rhs: *const T,
            rhs_cs: isize,
            rhs_rs: isize,
            alpha: T,
            beta: T,
            _conj_dst: bool,
            _conj_lhs: bool,
            _conj_rhs: bool,
            parallelism: gemm_common::Parallelism,
        ) {
            crate::gemm::gemm_basic_f16::<N, { MR_DIV_N * N }, NR, MR_DIV_N>(
                m,
                n,
                k,
                dst,
                dst_cs,
                dst_rs,
                read_dst,
                lhs,
                lhs_cs,
                lhs_rs,
                rhs,
                rhs_cs,
                rhs_rs,
                alpha,
                beta,
                &UKR,
                parallelism,
            );
        }
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    mod sse {
        use super::*;
        use gemm_f32::microkernel::sse::f32::*;
        const N: usize = 4;

        #[inline(never)]
        pub unsafe fn gemm_basic(
            m: usize,
            n: usize,
            k: usize,
            dst: *mut T,
            dst_cs: isize,
            dst_rs: isize,
            read_dst: bool,
            lhs: *const T,
            lhs_cs: isize,
            lhs_rs: isize,
            rhs: *const T,
            rhs_cs: isize,
            rhs_rs: isize,
            alpha: T,
            beta: T,
            _conj_dst: bool,
            _conj_lhs: bool,
            _conj_rhs: bool,
            parallelism: gemm_common::Parallelism,
        ) {
            gemm_basic_generic::<N, { MR_DIV_N * N }, NR, MR_DIV_N>(
                m,
                n,
                k,
                dst,
                dst_cs,
                dst_rs,
                read_dst,
                lhs,
                lhs_cs,
                lhs_rs,
                rhs,
                rhs_cs,
                rhs_rs,
                alpha,
                beta,
                &UKR,
                parallelism,
            );
        }
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    mod avx {
        use super::*;
        use gemm_f32::microkernel::avx::f32::*;
        const N: usize = 8;

        #[inline(never)]
        pub unsafe fn gemm_basic(
            m: usize,
            n: usize,
            k: usize,
            dst: *mut T,
            dst_cs: isize,
            dst_rs: isize,
            read_dst: bool,
            lhs: *const T,
            lhs_cs: isize,
            lhs_rs: isize,
            rhs: *const T,
            rhs_cs: isize,
            rhs_rs: isize,
            alpha: T,
            beta: T,
            _conj_dst: bool,
            _conj_lhs: bool,
            _conj_rhs: bool,
            parallelism: gemm_common::Parallelism,
        ) {
            gemm_basic_generic::<N, { MR_DIV_N * N }, NR, MR_DIV_N>(
                m,
                n,
                k,
                dst,
                dst_cs,
                dst_rs,
                read_dst,
                lhs,
                lhs_cs,
                lhs_rs,
                rhs,
                rhs_cs,
                rhs_rs,
                alpha,
                beta,
                &UKR,
                parallelism,
            );
        }
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    mod fma {
        use super::*;
        use gemm_f32::microkernel::fma::f32::*;
        const N: usize = 8;

        #[inline(never)]
        pub unsafe fn gemm_basic(
            m: usize,
            n: usize,
            k: usize,
            dst: *mut T,
            dst_cs: isize,
            dst_rs: isize,
            read_dst: bool,
            lhs: *const T,
            lhs_cs: isize,
            lhs_rs: isize,
            rhs: *const T,
            rhs_cs: isize,
            rhs_rs: isize,
            alpha: T,
            beta: T,
            _conj_dst: bool,
            _conj_lhs: bool,
            _conj_rhs: bool,
            parallelism: gemm_common::Parallelism,
        ) {
            gemm_basic_generic::<N, { MR_DIV_N * N }, NR, MR_DIV_N>(
                m,
                n,
                k,
                dst,
                dst_cs,
                dst_rs,
                read_dst,
                lhs,
                lhs_cs,
                lhs_rs,
                rhs,
                rhs_cs,
                rhs_rs,
                alpha,
                beta,
                &UKR,
                parallelism,
            );
        }
    }

    #[cfg(all(feature = "nightly", any(target_arch = "x86", target_arch = "x86_64")))]
    mod avx512f {
        use super::*;
        use gemm_f32::microkernel::avx512f::f32::*;
        const N: usize = 16;

        #[inline(never)]
        pub unsafe fn gemm_basic(
            m: usize,
            n: usize,
            k: usize,
            dst: *mut T,
            dst_cs: isize,
            dst_rs: isize,
            read_dst: bool,
            lhs: *const T,
            lhs_cs: isize,
            lhs_rs: isize,
            rhs: *const T,
            rhs_cs: isize,
            rhs_rs: isize,
            alpha: T,
            beta: T,
            _conj_dst: bool,
            _conj_lhs: bool,
            _conj_rhs: bool,
            parallelism: gemm_common::Parallelism,
        ) {
            gemm_basic_generic::<N, { MR_DIV_N * N }, NR, MR_DIV_N>(
                m,
                n,
                k,
                dst,
                dst_cs,
                dst_rs,
                read_dst,
                lhs,
                lhs_cs,
                lhs_rs,
                rhs,
                rhs_cs,
                rhs_rs,
                alpha,
                beta,
                &UKR,
                parallelism,
            );
        }
    }
}
