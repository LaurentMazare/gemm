use crate::{
    cache::{div_ceil, kernel_params, KernelParams, CACHE_INFO},
    gemv, gevv,
    microkernel::MicroKernelFn,
    pack_operands::{pack_lhs, pack_rhs},
    simd::Simd,
    Parallelism, Ptr,
};
use core::cell::RefCell;
use core::sync::atomic::{AtomicUsize, Ordering};
use dyn_stack::GlobalMemBuffer;
use dyn_stack::{DynStack, StackReq};
use num_traits::{One, Zero};

#[allow(non_camel_case_types)]
pub type c32 = num_complex::Complex32;
#[allow(non_camel_case_types)]
pub type c64 = num_complex::Complex64;

// https://rust-lang.github.io/hashbrown/src/crossbeam_utils/cache_padded.rs.html#128-130
pub const CACHELINE_ALIGN: usize = {
    #[cfg(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "powerpc64",
    ))]
    {
        128
    }
    #[cfg(any(
        target_arch = "arm",
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "riscv64",
    ))]
    {
        32
    }
    #[cfg(target_arch = "s390x")]
    {
        256
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "powerpc64",
        target_arch = "arm",
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "riscv64",
        target_arch = "s390x",
    )))]
    {
        64
    }
};

thread_local! {
    pub static L2_SLAB: RefCell<GlobalMemBuffer> = RefCell::new(GlobalMemBuffer::new(
        StackReq::new_aligned::<u8>(CACHE_INFO[1].cache_bytes, CACHELINE_ALIGN)
    ));
}

pub trait Conj: Copy {
    fn conj(self) -> Self;
}

impl Conj for f32 {
    #[inline(always)]
    fn conj(self) -> Self {
        self
    }
}
impl Conj for f64 {
    #[inline(always)]
    fn conj(self) -> Self {
        self
    }
}

impl Conj for c32 {
    #[inline(always)]
    fn conj(self) -> Self {
        c32 {
            re: self.re,
            im: -self.im,
        }
    }
}
impl Conj for c64 {
    #[inline(always)]
    fn conj(self) -> Self {
        c64 {
            re: self.re,
            im: -self.im,
        }
    }
}

pub const DEFAULT_THREADING_THRESHOLD: usize = 48 * 48 * 256;
pub const DEFAULT_RHS_PACKING_THRESHOLD: usize = 128;
pub const DEFAULT_LHS_PACKING_THRESHOLD_SINGLE_THREAD: usize = 8;
pub const DEFAULT_LHS_PACKING_THRESHOLD_MULTI_THREAD: usize = 16;

static THREADING_THRESHOLD: AtomicUsize = AtomicUsize::new(DEFAULT_THREADING_THRESHOLD);
static RHS_PACKING_THRESHOLD: AtomicUsize = AtomicUsize::new(DEFAULT_RHS_PACKING_THRESHOLD);
static LHS_PACKING_THRESHOLD_SINGLE_THREAD: AtomicUsize =
    AtomicUsize::new(DEFAULT_LHS_PACKING_THRESHOLD_SINGLE_THREAD);
static LHS_PACKING_THRESHOLD_MULTI_THREAD: AtomicUsize =
    AtomicUsize::new(DEFAULT_LHS_PACKING_THRESHOLD_MULTI_THREAD);

#[inline]
pub fn get_threading_threshold() -> usize {
    THREADING_THRESHOLD.load(Ordering::Relaxed)
}
#[inline]
pub fn set_threading_threshold(value: usize) {
    THREADING_THRESHOLD.store(value, Ordering::Relaxed);
}

#[inline]
pub fn get_rhs_packing_threshold() -> usize {
    RHS_PACKING_THRESHOLD.load(Ordering::Relaxed)
}
#[inline]
pub fn set_rhs_packing_threshold(value: usize) {
    RHS_PACKING_THRESHOLD.store(value.min(256), Ordering::Relaxed);
}

#[inline]
pub fn get_lhs_packing_threshold_single_thread() -> usize {
    LHS_PACKING_THRESHOLD_SINGLE_THREAD.load(Ordering::Relaxed)
}
#[inline]
pub fn set_lhs_packing_threshold_single_thread(value: usize) {
    LHS_PACKING_THRESHOLD_SINGLE_THREAD.store(value.min(256), Ordering::Relaxed);
}

#[inline]
pub fn get_lhs_packing_threshold_multi_thread() -> usize {
    LHS_PACKING_THRESHOLD_MULTI_THREAD.load(Ordering::Relaxed)
}
#[inline]
pub fn set_lhs_packing_threshold_multi_thread(value: usize) {
    LHS_PACKING_THRESHOLD_MULTI_THREAD.store(value.min(256), Ordering::Relaxed);
}

#[inline(always)]
pub fn par_for_each(n_threads: usize, func: impl Fn(usize) + Send + Sync) {
    rayon::scope(|s| {
        for thread_idx in 0..n_threads {
            let func = &func;
            s.spawn(move |_| func(thread_idx));
        }
    })
}

#[inline(always)]
pub unsafe fn gemm_basic_generic<
    S: Simd,
    T: Copy
        + Zero
        + One
        + Conj
        + Send
        + Sync
        + core::fmt::Debug
        + core::ops::Add<Output = T>
        + core::ops::Mul<Output = T>
        + core::cmp::PartialEq,
    const N: usize,
    const MR: usize,
    const NR: usize,
    const MR_DIV_N: usize,
>(
    simd: S,
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
    conj_dst: bool,
    conj_lhs: bool,
    conj_rhs: bool,
    mul_add: impl Copy + Fn(T, T, T) -> T,
    dispatcher: &[[MicroKernelFn<T>; NR]; MR_DIV_N],
    parallelism: Parallelism,
) {
    if m == 0 || n == 0 {
        return;
    }
    if !read_dst {
        alpha.set_zero();
    }

    if k == 0 {
        // dst = alpha * conj?(dst)

        if alpha.is_zero() {
            for j in 0..n {
                for i in 0..m {
                    *dst.offset(i as isize * dst_rs + j as isize * dst_cs) = T::zero();
                }
            }
            return;
        }

        if alpha.is_one() && !conj_dst {
            return;
        }

        if conj_dst {
            for j in 0..n {
                for i in 0..m {
                    let dst = dst.offset(i as isize * dst_rs + j as isize * dst_cs);
                    *dst = alpha * (*dst).conj();
                }
            }
        } else {
            for j in 0..n {
                for i in 0..m {
                    let dst = dst.offset(i as isize * dst_rs + j as isize * dst_cs);
                    *dst = alpha * *dst;
                }
            }
        }
        return;
    }

    if !conj_dst && !conj_lhs && !conj_rhs {
        if k <= 2 {
            gevv::gevv(
                simd, m, n, k, dst, dst_cs, dst_rs, lhs, lhs_cs, lhs_rs, rhs, rhs_cs, rhs_rs,
                alpha, beta, mul_add,
            );
            return;
        }
        if m <= 1 && rhs_cs.unsigned_abs() <= rhs_rs.unsigned_abs() {
            gemv::gemv(
                simd, n, m, k, dst, dst_rs, dst_cs, rhs, rhs_rs, rhs_cs, lhs, lhs_rs, lhs_cs,
                alpha, beta, mul_add,
            );
            return;
        }
        if n <= 1 && lhs_rs.unsigned_abs() <= lhs_cs.unsigned_abs() {
            gemv::gemv(
                simd, m, n, k, dst, dst_cs, dst_rs, lhs, lhs_cs, lhs_rs, rhs, rhs_cs, rhs_rs,
                alpha, beta, mul_add,
            );
            return;
        }
    }

    let KernelParams { kc, mc, nc } = if m <= 64 && n <= 64 {
        // skip expensive kernel_params call for small sizes
        let kc = k.min(512);
        let alloc = CACHE_INFO[1].cache_bytes / core::mem::size_of::<T>();
        let mc = (alloc / kc) / MR * MR;

        KernelParams {
            kc,
            mc,
            nc: div_ceil(n, NR) * NR,
        }
    } else {
        kernel_params(m, n, k, MR, NR, core::mem::size_of::<T>())
    };
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

    #[cfg(target_arch = "aarch64")]
    let do_pack_rhs = m > get_rhs_packing_threshold() * MR;

    // no need to pack if the lhs is already contiguous-ish
    #[cfg(not(target_arch = "aarch64"))]
    let do_pack_rhs = (rhs_rs.unsigned_abs() != 1 && m > 2 * MR)
        || (rhs_rs.unsigned_abs() == 1 && m > get_rhs_packing_threshold() * MR);

    let mut mem = if do_pack_rhs {
        Some(GlobalMemBuffer::new(StackReq::new_aligned::<T>(
            packed_rhs_stride * (nc / NR),
            simd_align,
        )))
    } else {
        None
    };

    let mut packed_rhs_storage = mem.as_mut().map(|mem| {
        let stack = DynStack::new(mem);
        stack
            .make_aligned_uninit::<T>(packed_rhs_stride * (nc / NR), simd_align)
            .0
    });

    let packed_rhs = packed_rhs_storage
        .as_mut()
        .map(|storage| storage.as_mut_ptr() as *mut T)
        .unwrap_or(core::ptr::null_mut());
    let packed_rhs = Ptr(packed_rhs);

    let packed_rhs_rs = if do_pack_rhs { NR as isize } else { rhs_rs };
    let packed_rhs_cs = if do_pack_rhs { 1 } else { rhs_cs };

    let mut col_outer = 0;
    while col_outer != n {
        let n_chunk = nc.min(n - col_outer);

        let mut alpha = alpha;
        let mut conj_dst = conj_dst;

        let mut depth_outer = 0;
        while depth_outer != k {
            let k_chunk = kc.min(k - depth_outer);
            let alpha_status = if alpha.is_zero() {
                0
            } else if alpha.is_one() {
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

            if do_pack_rhs {
                if n_threads <= 1 {
                    pack_rhs::<T, 1, NR, _>(
                        simd,
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
                            pack_rhs::<T, 1, NR, _>(
                                simd,
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

                        let packing_threshold = if n_threads == 1 {
                            get_lhs_packing_threshold_single_thread()
                        } else {
                            get_lhs_packing_threshold_multi_thread()
                        };
                        let do_pack_lhs =
                            (m_chunk % N != 0) || lhs_rs != 1 || n_chunk > packing_threshold * NR;
                        let packed_lhs_cs = if do_pack_lhs { MR as isize } else { lhs_cs };

                        if do_pack_lhs {
                            pack_lhs::<T, N, MR, _>(
                                simd,
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
                        }

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

                                func(
                                    m_chunk_inner,
                                    n_chunk_inner,
                                    k_chunk,
                                    dst.0,
                                    if do_pack_lhs {
                                        packed_lhs.wrapping_add(i * packed_lhs_stride).0
                                    } else {
                                        lhs.wrapping_offset(
                                            (row_outer + row_inner) as isize * lhs_rs
                                                + depth_outer as isize * lhs_cs,
                                        )
                                        .0
                                    },
                                    if do_pack_rhs {
                                        packed_rhs.wrapping_add(j * packed_rhs_stride).0
                                    } else {
                                        rhs.wrapping_offset(
                                            depth_outer as isize * rhs_rs
                                                + (col_outer + col_inner) as isize * rhs_cs,
                                        )
                                        .0
                                    },
                                    dst_cs,
                                    dst_rs,
                                    packed_lhs_cs,
                                    packed_rhs_rs,
                                    packed_rhs_cs,
                                    alpha,
                                    beta,
                                    alpha_status,
                                    conj_dst,
                                    conj_lhs,
                                    conj_rhs,
                                    if do_pack_lhs {
                                        packed_lhs.wrapping_add((i + 1) * packed_lhs_stride).0
                                    } else {
                                        lhs.wrapping_offset(
                                            (row_outer + row_inner + m_chunk_inner) as isize
                                                * lhs_rs
                                                + depth_outer as isize * lhs_cs,
                                        )
                                        .0
                                    },
                                );
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

            conj_dst = false;
            alpha.set_one();

            depth_outer += k_chunk;
        }
        col_outer += n_chunk;
    }
}

#[macro_export]
macro_rules! __inject_mod {
    ($module: ident, $ty: ident, $N: expr, $simd: ident) => {
        mod $module {
            use super::*;
            use crate::microkernel::$module::$ty::*;
            const N: usize = $N;

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
                conj_dst: bool,
                conj_lhs: bool,
                conj_rhs: bool,
                parallelism: $crate::Parallelism,
            ) {
                $crate::gemm::gemm_basic_generic::<_, T, N, { MR_DIV_N * N }, NR, MR_DIV_N>(
                    $crate::simd::$simd,
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
                    conj_dst,
                    conj_lhs,
                    conj_rhs,
                    |a, b, c| a * b + c,
                    &UKR,
                    parallelism,
                );
            }
        }
    };
}

#[macro_export]
macro_rules! __inject_mod_cplx {
    ($module: ident, $ty: ident, $N: expr, $simd: ident) => {
        paste::paste! {
            mod [<$module _cplx>] {
                use super::*;
                use crate::microkernel::$module::$ty::*;
                const N: usize = $N;

                #[inline(never)]
                pub unsafe fn gemm_basic_cplx(
                    m: usize,
                    n: usize,
                    k: usize,
                    dst: *mut num_complex::Complex<T>,
                    dst_cs: isize,
                    dst_rs: isize,
                    read_dst: bool,
                    lhs: *const num_complex::Complex<T>,
                    lhs_cs: isize,
                    lhs_rs: isize,
                    rhs: *const num_complex::Complex<T>,
                    rhs_cs: isize,
                    rhs_rs: isize,
                    alpha: num_complex::Complex<T>,
                    beta: num_complex::Complex<T>,
                    conj_dst: bool,
                    conj_lhs: bool,
                    conj_rhs: bool,
                    parallelism: $crate::Parallelism,
                    ) {
                    $crate::gemm::gemm_basic_generic::<_, _, N, { CPLX_MR_DIV_N * N }, CPLX_NR, CPLX_MR_DIV_N>(
                        $crate::simd::$simd,
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
                        conj_dst,
                        conj_lhs,
                        conj_rhs,
                        |a, b, c| a * b + c,
                        &CPLX_UKR,
                        parallelism,
                        );
                }
            }
        }
    };
}

#[macro_export]
macro_rules! gemm_def {
    ($ty: tt, $multiplier: expr) => {
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
            $crate::Parallelism,
        );

        fn init_gemm_fn() -> GemmTy {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            {
                #[cfg(feature = "nightly")]
                if $crate::feature_detected!("avx512f") {
                    return avx512f::gemm_basic;
                }
                if $crate::feature_detected!("fma") {
                    fma::gemm_basic
                } else if $crate::feature_detected!("avx") {
                    avx::gemm_basic
                } else if $crate::feature_detected!("sse") && $crate::feature_detected!("sse2") {
                    sse::gemm_basic
                } else {
                    scalar::gemm_basic
                }
            }

            #[cfg(target_arch = "aarch64")]
            {
                if $crate::feature_detected!("neon") {
                    neon::gemm_basic
                } else {
                    scalar::gemm_basic
                }
            }

            #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
            {
                simd128::gemm_basic
            }

            #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
            {
                scalar::gemm_basic
            }

            #[cfg(not(any(
                target_arch = "x86",
                target_arch = "x86_64",
                target_arch = "aarch64",
                target_arch = "wasm32"
            )))]
            {
                scalar::gemm_basic
            }
        }

        lazy_static::lazy_static! {
            pub static ref GEMM: GemmTy = init_gemm_fn();
        }

        $crate::__inject_mod!(scalar, $ty, 1, Scalar);

        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        $crate::__inject_mod!(sse, $ty, 2 * $multiplier, Sse);
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        $crate::__inject_mod!(avx, $ty, 4 * $multiplier, Avx);
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        $crate::__inject_mod!(fma, $ty, 4 * $multiplier, Fma);
        #[cfg(all(feature = "nightly", any(target_arch = "x86", target_arch = "x86_64")))]
        $crate::__inject_mod!(avx512f, $ty, 8 * $multiplier, Avx512f);

        #[cfg(target_arch = "aarch64")]
        $crate::__inject_mod!(neon, $ty, 2 * $multiplier, Neon);

        #[cfg(target_arch = "wasm32")]
        $crate::__inject_mod!(simd128, $ty, 2 * $multiplier, Simd128);
    };
}

#[macro_export]
macro_rules! gemm_cplx_def {
    ($ty: tt, $multiplier: expr) => {
        type GemmCplxTy = unsafe fn(
            usize,
            usize,
            usize,
            *mut num_complex::Complex<T>,
            isize,
            isize,
            bool,
            *const num_complex::Complex<T>,
            isize,
            isize,
            *const num_complex::Complex<T>,
            isize,
            isize,
            num_complex::Complex<T>,
            num_complex::Complex<T>,
            bool,
            bool,
            bool,
            $crate::Parallelism,
        );

        fn init_gemm_cplx_fn() -> GemmCplxTy {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            {
                #[cfg(feature = "nightly")]
                if $crate::feature_detected!("avx512f") {
                    return avx512f_cplx::gemm_basic_cplx;
                }
                if $crate::feature_detected!("fma") {
                    return fma_cplx::gemm_basic_cplx;
                }
            }

            scalar_cplx::gemm_basic_cplx
        }

        lazy_static::lazy_static! {
            pub static ref GEMM_CPLX: GemmCplxTy = init_gemm_cplx_fn();
        }

        $crate::__inject_mod_cplx!(scalar, $ty, 1, Scalar);

        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        $crate::__inject_mod_cplx!(fma, $ty, 2 * $multiplier, Fma);
        #[cfg(all(feature = "nightly", any(target_arch = "x86", target_arch = "x86_64")))]
        $crate::__inject_mod_cplx!(avx512f, $ty, 4 * $multiplier, Avx512f);
    };
}
