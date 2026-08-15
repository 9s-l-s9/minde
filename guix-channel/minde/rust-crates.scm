;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Channel-local Cargo crate sources for the packages in (minde packages).
;;;
;;; Guix's cargo-build-system resolves a package's Rust dependencies through
;;; a `lookup-cargo-inputs` table (see `define-cargo-inputs` in
;;; (guix build-system cargo)); Guix proper keeps that table in
;;; (gnu packages rust-crates). Crates a channel package needs at the exact
;;; versions its Cargo.lock pins are listed by the channel itself, in a
;;; module of the same shape, and referenced with
;;; `(cargo-inputs 'name #:module '(minde rust-crates))`.
;;;
;;; The `crate-source` definitions below are generated from the crate's own
;;; Cargo.lock (unpack the .crate archive from crates.io first):
;;;
;;;   guix import crate -f Cargo.lock shikane
;;;
;;; and pasted verbatim; only the `define-cargo-inputs` table at the end is
;;; hand-written. Regenerate the same way when bumping shikane. This module
;;; must load under a bare `guix pull` (no network, no vendor/), which it
;;; does: it only names crates.io URLs and hashes.

(define-module (minde rust-crates)
  #:use-module (guix build-system cargo)
  #:export (lookup-cargo-inputs))

(define rust-aho-corasick-1.1.3
  (crate-source "aho-corasick" "1.1.3"
                "05mrpkvdgp5d20y2p989f187ry9diliijgwrs254fs9s1m1x6q4f"))

(define rust-anstream-0.3.2
  (crate-source "anstream" "0.3.2"
                "0qzinx9c8zfq3xqpxzmlv6nrm3ymccr4n8gffkdmj31p50v4za0c"))

(define rust-anstyle-1.0.1
  (crate-source "anstyle" "1.0.1"
                "1kff80219d5rvvi407wky2zdlb0naxvbbg005s274pidbxfdlc1s"))

(define rust-anstyle-parse-0.2.1
  (crate-source "anstyle-parse" "0.2.1"
                "0cy38fbdlnmwyy6q8dn8dcfrpycwnpjkljsjqn3kmc40b7zp924k"))

(define rust-anstyle-query-1.0.0
  (crate-source "anstyle-query" "1.0.0"
                "0js9bgpqz21g0p2nm350cba1d0zfyixsma9lhyycic5sw55iv8aw"))

(define rust-anstyle-wincon-1.0.1
  (crate-source "anstyle-wincon" "1.0.1"
                "12714vwjf4c1wm3qf49m5vmd93qvq2nav6zpjc0bxbh3ayjby2hq"))

(define rust-autocfg-1.3.0
  (crate-source "autocfg" "1.3.0"
                "1c3njkfzpil03k92q0mij5y1pkhhfr4j3bf0h53bgl2vs85lsjqc"))

(define rust-base64-0.21.7
  (crate-source "base64" "0.21.7"
                "0rw52yvsk75kar9wgqfwgb414kvil1gn7mqkrhn9zf1537mpsacx"))

(define rust-bitflags-1.3.2
  (crate-source "bitflags" "1.3.2"
                "12ki6w8gn1ldq7yz9y680llwk5gmrhrzszaa17g1sbrw2r2qvwxy"))

(define rust-bitflags-2.5.0
  (crate-source "bitflags" "2.5.0"
                "1h91vdx1il069vdiiissj8ymzj130rbiic0dbs77yxjgjim9sjyg"))

(define rust-byteorder-1.4.3
  (crate-source "byteorder" "1.4.3"
                "0456lv9xi1a5bcm32arknf33ikv76p3fr9yzki4lb2897p2qkh8l"))

(define rust-calloop-0.13.0
  (crate-source "calloop" "0.13.0"
                "1v5zgidnhsyml403rzr7vm99f8q6r5bxq5gxyiqkr8lcapwa57dr"))

(define rust-calloop-wayland-source-0.3.0
  (crate-source "calloop-wayland-source" "0.3.0"
                "086x5mq16prrcwd9k6bw9an0sp8bj9l5daz4ziz5z4snf2c6m9lm"))

(define rust-cc-1.0.97
  (crate-source "cc" "1.0.97"
                "1d6rv3nk5q6lrr3mf7lifqpjr44slylsz3pw6pmn2k2cv1bm76h9"))

(define rust-cfg-if-1.0.0
  (crate-source "cfg-if" "1.0.0"
                "1za0vb97n4brpzpv8lsbnzmq5r8f2b0cpqqr0sy8h5bn751xxwds"))

(define rust-clap-4.3.8
  (crate-source "clap" "4.3.8"
                "04cjz9y98b3brmxmi47pjcjmmk2lxk1d2nrmccbkl9xlym842ffr"))

(define rust-clap-builder-4.3.8
  (crate-source "clap_builder" "4.3.8"
                "05q780vh2kq5zbkcpjaynpfbnrbppha42i5s6zglv4f27kfzny4s"))

(define rust-clap-derive-4.3.2
  (crate-source "clap_derive" "4.3.2"
                "0pw2bc8i7cxfrmwpa5wckx3fbw8s019nn7cgkv1yxmlsh4m2pkdq"))

(define rust-clap-lex-0.5.0
  (crate-source "clap_lex" "0.5.0"
                "06vcvpvp65qggc5agbirzqk2di00gxg6vazzc3qlwzkw70qxm9id"))

(define rust-colorchoice-1.0.0
  (crate-source "colorchoice" "1.0.0"
                "1ix7w85kwvyybwi2jdkl3yva2r2bvdcc3ka2grjfzfgrapqimgxc"))

(define rust-concurrent-queue-2.5.0
  (crate-source "concurrent-queue" "2.5.0"
                "0wrr3mzq2ijdkxwndhf79k952cp4zkz35ray8hvsxl96xrx1k82c"))

(define rust-crossbeam-utils-0.8.19
  (crate-source "crossbeam-utils" "0.8.19"
                "0iakrb1b8fjqrag7wphl94d10irhbh2fw1g444xslsywqyn3p3i4"))

(define rust-dlib-0.5.2
  (crate-source "dlib" "0.5.2"
                "04m4zzybx804394dnqs1blz241xcy480bdwf3w9p4k6c3l46031k"))

(define rust-doc-comment-0.3.3
  (crate-source "doc-comment" "0.3.3"
                "043sprsf3wl926zmck1bm7gw0jq50mb76lkpk49vasfr6ax1p97y"))

(define rust-downcast-rs-1.2.0
  (crate-source "downcast-rs" "1.2.0"
                "0l36kgxqd5djhqwf5abxjmgasdw8n0qsjvw3jdvhi91nj393ba4y"))

(define rust-either-1.11.0
  (crate-source "either" "1.11.0"
                "18l0cwyw18syl8b52syv6balql8mnwfyhihjqqllx5pms93iqz54"))

(define rust-env-logger-0.10.2
  (crate-source "env_logger" "0.10.2"
                "1005v71kay9kbz1d5907l0y7vh9qn2fqsp2yfgb8bjvin6m0bm2c"))

(define rust-errno-0.3.9
  (crate-source "errno" "0.3.9"
                "1fi0m0493maq1jygcf1bya9cymz2pc1mqxj26bdv7yjd37v5qk2k"))

(define rust-futures-0.3.30
  (crate-source "futures" "0.3.30"
                "1c04g14bccmprwsvx2j9m2blhwrynq7vhl151lsvcv4gi0b6jp34"))

(define rust-futures-channel-0.3.30
  (crate-source "futures-channel" "0.3.30"
                "0y6b7xxqdjm9hlcjpakcg41qfl7lihf6gavk8fyqijsxhvbzgj7a"))

(define rust-futures-core-0.3.30
  (crate-source "futures-core" "0.3.30"
                "07aslayrn3lbggj54kci0ishmd1pr367fp7iks7adia1p05miinz"))

(define rust-futures-executor-0.3.30
  (crate-source "futures-executor" "0.3.30"
                "07dh08gs9vfll2h36kq32q9xd86xm6lyl9xikmmwlkqnmrrgqxm5"))

(define rust-futures-io-0.3.30
  (crate-source "futures-io" "0.3.30"
                "1hgh25isvsr4ybibywhr4dpys8mjnscw4wfxxwca70cn1gi26im4"))

(define rust-futures-macro-0.3.30
  (crate-source "futures-macro" "0.3.30"
                "1b49qh9d402y8nka4q6wvvj0c88qq91wbr192mdn5h54nzs0qxc7"))

(define rust-futures-sink-0.3.30
  (crate-source "futures-sink" "0.3.30"
                "1dag8xyyaya8n8mh8smx7x6w2dpmafg2din145v973a3hw7f1f4z"))

(define rust-futures-task-0.3.30
  (crate-source "futures-task" "0.3.30"
                "013h1724454hj8qczp8vvs10qfiqrxr937qsrv6rhii68ahlzn1q"))

(define rust-futures-timer-3.0.3
  (crate-source "futures-timer" "3.0.3"
                "094vw8k37djpbwv74bwf2qb7n6v6ghif4myss6smd6hgyajb127j"))

(define rust-futures-util-0.3.30
  (crate-source "futures-util" "0.3.30"
                "0j0xqhcir1zf2dcbpd421kgw6wvsk0rpxflylcysn1rlp3g02r1x"))

(define rust-fxhash-0.2.1
  (crate-source "fxhash" "0.2.1"
                "037mb9ichariqi45xm6mz0b11pa92gj38ba0409z3iz239sns6y3"))

(define rust-glob-0.3.1
  (crate-source "glob" "0.3.1"
                "16zca52nglanv23q5qrwd5jinw3d3as5ylya6y1pbx47vkxvrynj"))

(define rust-hashbrown-0.12.3
  (crate-source "hashbrown" "0.12.3"
                "1268ka4750pyg2pbgsr43f0289l5zah4arir2k4igx5a8c6fg7la"))

(define rust-heck-0.4.1
  (crate-source "heck" "0.4.1"
                "1a7mqsnycv5z4z5vnv1k34548jzmc0ajic7c1j8jsaspnhw5ql4m"))

(define rust-hermit-abi-0.3.9
  (crate-source "hermit-abi" "0.3.9"
                "092hxjbjnq5fmz66grd9plxd0sh6ssg5fhgwwwqbrzgzkjwdycfj"))

(define rust-hopcroft-karp-0.2.1
  (crate-source "hopcroft-karp" "0.2.1"
                "0bknn9622mx2aw73jvz3haw4wkkmvav08df2s7snyxinm6dksbx7"))

(define rust-humantime-2.1.0
  (crate-source "humantime" "2.1.0"
                "1r55pfkkf5v0ji1x6izrjwdq9v6sc7bv99xj6srywcar37xmnfls"))

(define rust-indexmap-1.9.3
  (crate-source "indexmap" "1.9.3"
                "16dxmy7yvk51wvnih3a3im6fp5lmx0wx76i03n06wyak6cwhw1xx"))

(define rust-is-terminal-0.4.12
  (crate-source "is-terminal" "0.4.12"
                "12vk6g0f94zlxl6mdh5gc4jdjb469n9k9s7y3vb0iml05gpzagzj"))

(define rust-itertools-0.12.1
  (crate-source "itertools" "0.12.1"
                "0s95jbb3ndj1lvfxyq5wanc0fm0r6hg6q4ngb92qlfdxvci10ads"))

(define rust-libc-0.2.155
  (crate-source "libc" "0.2.155"
                "0z44c53z54znna8n322k5iwg80arxxpdzjj5260pxxzc9a58icwp"))

(define rust-libloading-0.8.0
  (crate-source "libloading" "0.8.0"
                "1jyq4bzq1n3c7rmblcwnd0xvxr553zmrikr83ch0arbpjn7k306m"))

(define rust-linux-raw-sys-0.4.13
  ;; TODO REVIEW: Check bundled sources.
  (crate-source "linux-raw-sys" "0.4.13"
                "172k2c6422gsc914ig8rh99mb9yc7siw6ikc3d9xw1k7vx0s3k81"))

(define rust-log-0.4.21
  (crate-source "log" "0.4.21"
                "074hldq1q8rlzq2s2qa8f25hj4s3gpw71w64vdwzjd01a4g8rvch"))

(define rust-memchr-2.7.2
  (crate-source "memchr" "2.7.2"
                "07bcqxb0vx4ji0648ny5xsicjnpma95x1n07v7mi7jrhsz2l11kc"))

(define rust-once-cell-1.18.0
  (crate-source "once_cell" "1.18.0"
                "0vapcd5ambwck95wyz3ymlim35jirgnqn9a0qmi19msymv95v2yx"))

(define rust-pin-project-lite-0.2.14
  (crate-source "pin-project-lite" "0.2.14"
                "00nx3f04agwjlsmd3mc5rx5haibj2v8q9b52b0kwn63wcv4nz9mx"))

(define rust-pin-utils-0.1.0
  (crate-source "pin-utils" "0.1.0"
                "117ir7vslsl2z1a7qzhws4pd01cg2d3338c47swjyvqv2n60v1wb"))

(define rust-pkg-config-0.3.27
  (crate-source "pkg-config" "0.3.27"
                "0r39ryh1magcq4cz5g9x88jllsnxnhcqr753islvyk4jp9h2h1r6"))

(define rust-polling-3.7.0
  (crate-source "polling" "3.7.0"
                "1qvvccdbk49xmrwic5ljikgjvf8zrxlaf0lx44jfymj46k7r6m34"))

(define rust-proc-macro2-1.0.82
  (crate-source "proc-macro2" "1.0.82"
                "06qk88hbf6wg4v1i961zibhjz512873jwkz3myx1z82ip6dd9lwa"))

(define rust-quick-xml-0.31.0
  (crate-source "quick-xml" "0.31.0"
                "0cravqanylzh5cq2v6hzlfqgxcid5nrp2snnb3pf4m0and2a610h"))

(define rust-quote-1.0.36
  (crate-source "quote" "1.0.36"
                "19xcmh445bg6simirnnd4fvkmp6v2qiwxh5f6rw4a70h76pnm9qg"))

(define rust-regex-1.10.4
  (crate-source "regex" "1.10.4"
                "0k5sb0h2mkwf51ab0gvv3x38jp1q7wgxf63abfbhi0wwvvgxn5y1"))

(define rust-regex-automata-0.4.6
  (crate-source "regex-automata" "0.4.6"
                "1spaq7y4im7s56d1gxa2hi4hzf6dwswb1bv8xyavzya7k25kpf46"))

(define rust-regex-syntax-0.8.3
  (crate-source "regex-syntax" "0.8.3"
                "0mhzkm1pkqg6y53xv056qciazlg47pq0czqs94cn302ckvi49bdd"))

(define rust-relative-path-1.9.3
  (crate-source "relative-path" "1.9.3"
                "1limlh8fzwi21g0473fqzd6fln9iqkwvzp3816bxi31pkilz6fds"))

(define rust-ron-0.8.1
  (crate-source "ron" "0.8.1"
                "154w53s895yxdfg7rn87c6f6x4yncc535x1x31zpcj7p0pzpw7xr"))

(define rust-rstest-0.19.0
  (crate-source "rstest" "0.19.0"
                "0c43nsxpm1b74jxc73xwg94is6bwqvfzkrr1xbqyx7j7l791clwx"))

(define rust-rstest-macros-0.19.0
  (crate-source "rstest_macros" "0.19.0"
                "09ackagv8kc2v4xy0s7blyg4agij9bz9pbb31l5h4rqzrirdza84"))

(define rust-rustc-version-0.4.0
  (crate-source "rustc_version" "0.4.0"
                "0rpk9rcdk405xhbmgclsh4pai0svn49x35aggl4nhbkd4a2zb85z"))

(define rust-rustix-0.38.34
  (crate-source "rustix" "0.38.34"
                "03vkqa2ism7q56rkifyy8mns0wwqrk70f4i4fd53r97p8b05xp3h"))

(define rust-scoped-tls-1.0.1
  (crate-source "scoped-tls" "1.0.1"
                "15524h04mafihcvfpgxd8f4bgc3k95aclz8grjkg9a0rxcvn9kz1"))

(define rust-semver-1.0.23
  (crate-source "semver" "1.0.23"
                "12wqpxfflclbq4dv8sa6gchdh92ahhwn4ci1ls22wlby3h57wsb1"))

(define rust-serde-1.0.201
  (crate-source "serde" "1.0.201"
                "0g1nrz2s6l36na6gdbph8k07xf9h5p3s6f0s79sy8a8nxpmiq3vq"))

(define rust-serde-derive-1.0.201
  (crate-source "serde_derive" "1.0.201"
                "0r98v8h47s7zhml7gz0sl6wv82vyzh1hv27f1g0g35lp1f9hbr65"))

(define rust-slab-0.4.9
  (crate-source "slab" "0.4.9"
                "0rxvsgir0qw5lkycrqgb1cxsvxzjv9bmx73bk5y42svnzfba94lg"))

(define rust-smallvec-1.10.0
  (crate-source "smallvec" "1.10.0"
                "1q2k15fzxgwjpcdv3f323w24rbbfyv711ayz85ila12lg7zbw1x5"))

(define rust-snafu-0.7.5
  (crate-source "snafu" "0.7.5"
                "1mj2j2gfbf8mm1hr02zrbrqrh2zp01f61xgkx0lpln2w0ankgpp4"))

(define rust-snafu-derive-0.7.5
  (crate-source "snafu-derive" "0.7.5"
                "1gzy9rzggs090zf7hfvgp4lm1glrmg9qzh796686jnq7bxk7j04r"))

(define rust-strsim-0.10.0
  (crate-source "strsim" "0.10.0"
                "08s69r4rcrahwnickvi0kq49z524ci50capybln83mg6b473qivk"))

(define rust-syn-1.0.109
  (crate-source "syn" "1.0.109"
                "0ds2if4600bd59wsv7jjgfkayfzy3hnazs394kz6zdkmna8l3dkj"))

(define rust-syn-2.0.61
  (crate-source "syn" "2.0.61"
                "1j8zhf5mmd2l5niwhiniw5wcp9v6fbd4a61v6rbfhsm5rf6fv4y9"))

(define rust-termcolor-1.4.1
  (crate-source "termcolor" "1.4.1"
                "0mappjh3fj3p2nmrg4y7qv94rchwi9mzmgmfflr8p2awdj7lyy86"))

(define rust-thiserror-1.0.60
  (crate-source "thiserror" "1.0.60"
                "067wi7pb1zn9jhhk82w0ppmvjwa00nwkp4m9j77rvpaqra1r17jp"))

(define rust-thiserror-impl-1.0.60
  (crate-source "thiserror-impl" "1.0.60"
                "0945q2hk1rqdzjz2zqakxbddwm4h26k5c0wdncdarhvfq10h0iz2"))

(define rust-toml-0.5.11
  (crate-source "toml" "0.5.11"
                "0d2266nx8b3n22c7k24x4428z6di8n83a9n466jm7a2hipfz1xzl"))

(define rust-tracing-0.1.40
  (crate-source "tracing" "0.1.40"
                "1vv48dac9zgj9650pg2b4d0j3w6f3x9gbggf43scq5hrlysklln3"))

(define rust-tracing-core-0.1.32
  (crate-source "tracing-core" "0.1.32"
                "0m5aglin3cdwxpvbg6kz0r9r0k31j48n0kcfwsp6l49z26k3svf0"))

(define rust-unicode-ident-1.0.12
  (crate-source "unicode-ident" "1.0.12"
                "0jzf1znfpb2gx8nr8mvmyqs1crnv79l57nxnbiszc7xf7ynbjm1k"))

(define rust-utf8parse-0.2.1
  (crate-source "utf8parse" "0.2.1"
                "02ip1a0az0qmc2786vxk2nqwsgcwf17d3a38fkf0q7hrmwh9c6vi"))

(define rust-wayland-backend-0.3.3
  (crate-source "wayland-backend" "0.3.3"
                "0h4s8nfrl1q8xys1409lfwkb70cdh81c0pvzr1s69mwhrrhzll4x"))

(define rust-wayland-client-0.31.2
  (crate-source "wayland-client" "0.31.2"
                "07rzml07li3bi4nnqx4i2rfj3xkifzxp1d6cd1kflb2wjgp9dyw2"))

(define rust-wayland-protocols-0.31.2
  (crate-source "wayland-protocols" "0.31.2"
                "1x310l1p6p3p3l76nl1l2yava9408dy77s605917zadlp1jz70cg"))

(define rust-wayland-protocols-wlr-0.2.0
  (crate-source "wayland-protocols-wlr" "0.2.0"
                "1mjww9psk2nc5hm2q4s3qas30rbzfg1sb6qgw518fbbcdfvn27xd"))

(define rust-wayland-scanner-0.31.1
  (crate-source "wayland-scanner" "0.31.1"
                "10y2nq076x4zml8wc5bw75560rwvrsfpi35mdyc02w1854lsdcv3"))

(define rust-wayland-sys-0.31.1
  ;; TODO REVIEW: Check bundled sources.
  (crate-source "wayland-sys" "0.31.1"
                "1bxpwamgagpxa8p9m798gd3g6rwj2m4sbdvc49zx05jjzzmci80m"))

(define rust-winapi-util-0.1.8
  (crate-source "winapi-util" "0.1.8"
                "0svcgddd2rw06mj4r76gj655qsa1ikgz3d3gzax96fz7w62c6k2d"))

(define rust-windows-sys-0.48.0
  ;; TODO REVIEW: Check bundled sources.
  (crate-source "windows-sys" "0.48.0"
                "1aan23v5gs7gya1lc46hqn9mdh8yph3fhxmhxlw36pn6pqc28zb7"))

(define rust-windows-sys-0.52.0
  ;; TODO REVIEW: Check bundled sources.
  (crate-source "windows-sys" "0.52.0"
                "0gd3v4ji88490zgb6b5mq5zgbvwv7zx1ibn8v3x83rwcdbryaar8"))

(define rust-windows-targets-0.48.5
  (crate-source "windows-targets" "0.48.5"
                "034ljxqshifs1lan89xwpcy1hp0lhdh4b5n0d2z4fwjx2piacbws"))

(define rust-windows-targets-0.52.5
  (crate-source "windows-targets" "0.52.5"
                "1sz7jrnkygmmlj1ia8fk85wbyil450kq5qkh5qh9sh2rcnj161vg"))

(define rust-windows-aarch64-gnullvm-0.48.5
  (crate-source "windows_aarch64_gnullvm" "0.48.5"
                "1n05v7qblg1ci3i567inc7xrkmywczxrs1z3lj3rkkxw18py6f1b"))

(define rust-windows-aarch64-gnullvm-0.52.5
  (crate-source "windows_aarch64_gnullvm" "0.52.5"
                "0qrjimbj67nnyn7zqy15mzzmqg0mn5gsr2yciqjxm3cb3vbyx23h"))

(define rust-windows-aarch64-msvc-0.48.5
  (crate-source "windows_aarch64_msvc" "0.48.5"
                "1g5l4ry968p73g6bg6jgyvy9lb8fyhcs54067yzxpcpkf44k2dfw"))

(define rust-windows-aarch64-msvc-0.52.5
  (crate-source "windows_aarch64_msvc" "0.52.5"
                "1dmga8kqlmln2ibckk6mxc9n59vdg8ziqa2zr8awcl720hazv1cr"))

(define rust-windows-i686-gnu-0.48.5
  (crate-source "windows_i686_gnu" "0.48.5"
                "0gklnglwd9ilqx7ac3cn8hbhkraqisd0n83jxzf9837nvvkiand7"))

(define rust-windows-i686-gnu-0.52.5
  (crate-source "windows_i686_gnu" "0.52.5"
                "0w4np3l6qwlra9s2xpflqrs60qk1pz6ahhn91rr74lvdy4y0gfl8"))

(define rust-windows-i686-gnullvm-0.52.5
  (crate-source "windows_i686_gnullvm" "0.52.5"
                "1s9f4gff0cixd86mw3n63rpmsm4pmr4ffndl6s7qa2h35492dx47"))

(define rust-windows-i686-msvc-0.48.5
  (crate-source "windows_i686_msvc" "0.48.5"
                "01m4rik437dl9rdf0ndnm2syh10hizvq0dajdkv2fjqcywrw4mcg"))

(define rust-windows-i686-msvc-0.52.5
  (crate-source "windows_i686_msvc" "0.52.5"
                "1gw7fklxywgpnwbwg43alb4hm0qjmx72hqrlwy5nanrxs7rjng6v"))

(define rust-windows-x86-64-gnu-0.48.5
  (crate-source "windows_x86_64_gnu" "0.48.5"
                "13kiqqcvz2vnyxzydjh73hwgigsdr2z1xpzx313kxll34nyhmm2k"))

(define rust-windows-x86-64-gnu-0.52.5
  (crate-source "windows_x86_64_gnu" "0.52.5"
                "1n8p2mcf3lw6300k77a0knksssmgwb9hynl793mhkzyydgvlchjf"))

(define rust-windows-x86-64-gnullvm-0.48.5
  (crate-source "windows_x86_64_gnullvm" "0.48.5"
                "1k24810wfbgz8k48c2yknqjmiigmql6kk3knmddkv8k8g1v54yqb"))

(define rust-windows-x86-64-gnullvm-0.52.5
  (crate-source "windows_x86_64_gnullvm" "0.52.5"
                "15n56jrh4s5bz66zimavr1rmcaw6wa306myrvmbc6rydhbj9h8l5"))

(define rust-windows-x86-64-msvc-0.48.5
  (crate-source "windows_x86_64_msvc" "0.48.5"
                "0f4mdp895kkjh9zv8dxvn4pc10xr7839lf5pa9l0193i2pkgr57d"))

(define rust-windows-x86-64-msvc-0.52.5
  (crate-source "windows_x86_64_msvc" "0.52.5"
                "1w1bn24ap8dp9i85s8mlg8cim2bl2368bd6qyvm0xzqvzmdpxi5y"))

(define rust-xdg-2.5.2
  (crate-source "xdg" "2.5.2"
                "0im5nzmywxjgm2pmb48k0cc9hkalarz57f1d9d0x4lvb6cj76fr1"))


(define-cargo-inputs lookup-cargo-inputs
  (shikane =>
   (list rust-aho-corasick-1.1.3 rust-anstream-0.3.2 rust-anstyle-1.0.1
         rust-anstyle-parse-0.2.1 rust-anstyle-query-1.0.0
         rust-anstyle-wincon-1.0.1 rust-autocfg-1.3.0 rust-base64-0.21.7
         rust-bitflags-1.3.2 rust-bitflags-2.5.0 rust-byteorder-1.4.3
         rust-calloop-0.13.0 rust-calloop-wayland-source-0.3.0
         rust-cc-1.0.97 rust-cfg-if-1.0.0 rust-clap-4.3.8
         rust-clap-builder-4.3.8 rust-clap-derive-4.3.2 rust-clap-lex-0.5.0
         rust-colorchoice-1.0.0 rust-concurrent-queue-2.5.0
         rust-crossbeam-utils-0.8.19 rust-dlib-0.5.2 rust-doc-comment-0.3.3
         rust-downcast-rs-1.2.0 rust-either-1.11.0 rust-env-logger-0.10.2
         rust-errno-0.3.9 rust-futures-0.3.30 rust-futures-channel-0.3.30
         rust-futures-core-0.3.30 rust-futures-executor-0.3.30
         rust-futures-io-0.3.30 rust-futures-macro-0.3.30
         rust-futures-sink-0.3.30 rust-futures-task-0.3.30
         rust-futures-timer-3.0.3 rust-futures-util-0.3.30 rust-fxhash-0.2.1
         rust-glob-0.3.1 rust-hashbrown-0.12.3 rust-heck-0.4.1
         rust-hermit-abi-0.3.9 rust-hopcroft-karp-0.2.1 rust-humantime-2.1.0
         rust-indexmap-1.9.3 rust-is-terminal-0.4.12 rust-itertools-0.12.1
         rust-libc-0.2.155 rust-libloading-0.8.0 rust-linux-raw-sys-0.4.13
         rust-log-0.4.21 rust-memchr-2.7.2 rust-once-cell-1.18.0
         rust-pin-project-lite-0.2.14 rust-pin-utils-0.1.0
         rust-pkg-config-0.3.27 rust-polling-3.7.0 rust-proc-macro2-1.0.82
         rust-quick-xml-0.31.0 rust-quote-1.0.36 rust-regex-1.10.4
         rust-regex-automata-0.4.6 rust-regex-syntax-0.8.3
         rust-relative-path-1.9.3 rust-ron-0.8.1 rust-rstest-0.19.0
         rust-rstest-macros-0.19.0 rust-rustc-version-0.4.0
         rust-rustix-0.38.34 rust-scoped-tls-1.0.1 rust-semver-1.0.23
         rust-serde-1.0.201 rust-serde-derive-1.0.201 rust-slab-0.4.9
         rust-smallvec-1.10.0 rust-snafu-0.7.5 rust-snafu-derive-0.7.5
         rust-strsim-0.10.0 rust-syn-1.0.109 rust-syn-2.0.61
         rust-termcolor-1.4.1 rust-thiserror-1.0.60
         rust-thiserror-impl-1.0.60 rust-toml-0.5.11 rust-tracing-0.1.40
         rust-tracing-core-0.1.32 rust-unicode-ident-1.0.12
         rust-utf8parse-0.2.1 rust-wayland-backend-0.3.3
         rust-wayland-client-0.31.2 rust-wayland-protocols-0.31.2
         rust-wayland-protocols-wlr-0.2.0 rust-wayland-scanner-0.31.1
         rust-wayland-sys-0.31.1 rust-winapi-util-0.1.8
         rust-windows-sys-0.48.0 rust-windows-sys-0.52.0
         rust-windows-targets-0.48.5 rust-windows-targets-0.52.5
         rust-windows-aarch64-gnullvm-0.48.5
         rust-windows-aarch64-gnullvm-0.52.5
         rust-windows-aarch64-msvc-0.48.5 rust-windows-aarch64-msvc-0.52.5
         rust-windows-i686-gnu-0.48.5 rust-windows-i686-gnu-0.52.5
         rust-windows-i686-gnullvm-0.52.5 rust-windows-i686-msvc-0.48.5
         rust-windows-i686-msvc-0.52.5 rust-windows-x86-64-gnu-0.48.5
         rust-windows-x86-64-gnu-0.52.5 rust-windows-x86-64-gnullvm-0.48.5
         rust-windows-x86-64-gnullvm-0.52.5 rust-windows-x86-64-msvc-0.48.5
         rust-windows-x86-64-msvc-0.52.5 rust-xdg-2.5.2)))
