// Forza Horizon 6 - Türkçe Dublaj kurulum programı (Steam + Xbox/Microsoft Store)
// .NET Framework 4 (C# 5) ile derlenir: csc /target:winexe
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Windows.Forms;
using Microsoft.Win32;

[assembly: System.Reflection.AssemblyTitle("FH6 Türkçe Dublaj Kurulumu")]
[assembly: System.Reflection.AssemblyProduct("FH6 Türkçe Dublaj")]
[assembly: System.Reflection.AssemblyVersion("1.0.0.0")]

static class Program
{
    [STAThread]
    static void Main()
    {
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new AnaForm());
    }
}

class Oyun
{
    public string Tur;
    public string Banka;   // ...\media\Audio\FMODBanks
    public bool Kurulu { get { return Directory.Exists(Path.Combine(Banka, AnaForm.YEDEK)); } }
    public override string ToString()
    {
        return (Kurulu ? "[Türkçe kurulu]  " : "") + Tur + "  —  " + Banka;
    }
}

class AnaForm : Form
{
    public const string YEDEK = "TR_Dublaj_Yedek";
    const string KONTROL = "VO_Gameplay_Mei_EN.assets.bank";

    static readonly Color Lacivert = Color.FromArgb(13, 59, 128);
    static readonly Color Mavi = Color.FromArgb(25, 118, 210);
    static readonly Color MaviAcik = Color.FromArgb(66, 150, 235);
    static readonly Color MaviKoyu = Color.FromArgb(16, 90, 170);
    static readonly Color Zemin = Color.FromArgb(241, 243, 246);
    static readonly Color Gri = Color.FromArgb(222, 226, 231);
    static readonly Color GriKoyu = Color.FromArgb(200, 205, 212);
    static readonly Color Yazi = Color.FromArgb(33, 37, 41);
    static readonly Color YaziSoluk = Color.FromArgb(108, 117, 125);

    ListBox liste;
    Button btnKur, btnKaldir, btnSec, btnTara;
    Panel pb;
    int pbDeger, pbMax = 1;
    Label durum;
    string veri;
    float s = 1f; // DPI ölçeği (sadece elle çizilen öğeler için)

    public AnaForm()
    {
        SuspendLayout();
        AutoScaleDimensions = new SizeF(96F, 96F);
        AutoScaleMode = AutoScaleMode.Dpi;
        Text = "Forza Horizon 6 — Türkçe Dublaj Kurulumu";
        ClientSize = new Size(760, 580);
        FormBorderStyle = FormBorderStyle.FixedSingle;
        MaximizeBox = false;
        StartPosition = FormStartPosition.CenterScreen;
        BackColor = Zemin;
        Font = new Font("Segoe UI", 9.75f);
        DoubleBuffered = true;
        using (var g = CreateGraphics()) s = g.DpiX / 96f;
        veri = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "Veri");

        // Üst şerit
        var ust = new Panel { Left = 0, Top = 0, Width = 760, Height = 104 };
        ust.Paint += UstCiz;
        Controls.Add(ust);

        var bilgi = new Label
        {
            Left = 24, Top = 118, Width = 712, Height = 24, ForeColor = YaziSoluk,
            Text = "Orijinal İngilizce ses dosyaları yedeklenir; istediğiniz zaman \"Kaldır\" ile geri dönebilirsiniz."
        };
        Controls.Add(bilgi);

        Controls.Add(Baslik("1", "Oyun kurulumunu seçin", 154));

        var kart = new Panel { Left = 24, Top = 190, Width = 712, Height = 152, BackColor = Color.White, Padding = new Padding(1) };
        kart.Paint += delegate (object o, PaintEventArgs e) { using (var p = new Pen(GriKoyu)) e.Graphics.DrawRectangle(p, 0, 0, kart.Width - 1, kart.Height - 1); };
        liste = new ListBox
        {
            Dock = DockStyle.Fill, BorderStyle = BorderStyle.None, DrawMode = DrawMode.OwnerDrawFixed,
            ItemHeight = (int)(60 * s), IntegralHeight = false, BackColor = Color.White
        };
        liste.DrawItem += OgeCiz;
        liste.SelectedIndexChanged += delegate { liste.Invalidate(); };
        kart.Controls.Add(liste);
        Controls.Add(kart);

        btnTara = Dugme("Yeniden ara", 24, 352, 170, 38, false);
        btnSec = Dugme("Oyun klasörünü kendim seçeyim", 202, 352, 270, 38, false);
        Controls.Add(btnTara); Controls.Add(btnSec);

        Controls.Add(Baslik("2", "Kurun", 408));

        btnKur = Dugme("TÜRKÇE DUBLAJI KUR", 24, 444, 470, 58, true);
        btnKur.Font = new Font("Segoe UI Semibold", 12.5f);
        btnKaldir = Dugme("Kaldır", 506, 444, 230, 58, false);
        btnKaldir.Font = new Font("Segoe UI Semibold", 11.5f);
        Controls.Add(btnKur); Controls.Add(btnKaldir);

        pb = new Panel { Left = 24, Top = 516, Width = 712, Height = 8, BackColor = Gri };
        pb.Paint += delegate (object o, PaintEventArgs e)
        {
            int w = (int)((long)pb.Width * pbDeger / Math.Max(1, pbMax));
            using (var b = new SolidBrush(Mavi)) e.Graphics.FillRectangle(b, 0, 0, w, pb.Height);
        };
        Controls.Add(pb);

        durum = new Label { Left = 24, Top = 530, Width = 712, Height = 44, ForeColor = Yazi };
        Controls.Add(durum);

        btnTara.Click += delegate { Tara(); };
        btnSec.Click += delegate { ElleSec(); };
        btnKur.Click += delegate { Calistir(true); };
        btnKaldir.Click += delegate { Calistir(false); };
        Load += delegate { Tara(); };
        ResumeLayout(false);
    }

    void UstCiz(object sender, PaintEventArgs e)
    {
        var p = (Panel)sender;
        var g = e.Graphics;
        g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
        using (var b = new System.Drawing.Drawing2D.LinearGradientBrush(p.ClientRectangle, Lacivert, Mavi, 0f))
            g.FillRectangle(b, p.ClientRectangle);
        // dekoratif şeritler
        using (var b = new SolidBrush(Color.FromArgb(28, 255, 255, 255)))
        {
            g.FillPolygon(b, new[] { new PointF(p.Width * 0.62f, 0), new PointF(p.Width * 0.74f, 0), new PointF(p.Width * 0.60f, p.Height), new PointF(p.Width * 0.48f, p.Height) });
            g.FillPolygon(b, new[] { new PointF(p.Width * 0.80f, 0), new PointF(p.Width * 0.86f, 0), new PointF(p.Width * 0.72f, p.Height), new PointF(p.Width * 0.66f, p.Height) });
        }
        using (var f1 = new Font("Segoe UI", 20f, FontStyle.Bold))
        using (var f2 = new Font("Segoe UI", 11f))
        {
            TextRenderer.DrawText(g, "FORZA HORIZON 6", f1, new Point((int)(24 * s), (int)(16 * s)), Color.White, TextFormatFlags.NoPadding);
            TextRenderer.DrawText(g, "Türkçe Dublaj Kurulumu", f2, new Point((int)(26 * s), (int)(62 * s)), Color.FromArgb(200, 225, 255), TextFormatFlags.NoPadding);
        }
        using (var f3 = new Font("Segoe UI", 8.5f))
            TextRenderer.DrawText(g, "Steam  •  Xbox / Game Pass", f3, new Rectangle(0, (int)(70 * s), p.Width - (int)(24 * s), (int)(20 * s)), Color.FromArgb(185, 210, 245), TextFormatFlags.Right);
    }

    Control Baslik(string no, string metin, int top)
    {
        var p = new Panel { Left = 24, Top = top, Width = 712, Height = 30 };
        p.Paint += delegate (object o, PaintEventArgs e)
        {
            var g = e.Graphics;
            g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
            int d = (int)(26 * s);
            using (var b = new SolidBrush(Mavi)) g.FillEllipse(b, 0, (p.Height - d) / 2, d, d);
            using (var f = new Font("Segoe UI", 10f, FontStyle.Bold))
                TextRenderer.DrawText(g, no, f, new Rectangle(0, (p.Height - d) / 2, d, d), Color.White, TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter);
            using (var f = new Font("Segoe UI Semibold", 12f))
                TextRenderer.DrawText(g, metin, f, new Point(d + (int)(10 * s), 0), Yazi, TextFormatFlags.NoPadding);
        };
        return p;
    }

    Button Dugme(string metin, int x, int y, int w, int h, bool birincil)
    {
        var b = new Button
        {
            Text = metin, Left = x, Top = y, Width = w, Height = h,
            FlatStyle = FlatStyle.Flat, Cursor = Cursors.Hand, UseVisualStyleBackColor = false,
            BackColor = birincil ? Mavi : Gri, ForeColor = birincil ? Color.White : Yazi
        };
        b.FlatAppearance.BorderSize = 0;
        b.FlatAppearance.MouseOverBackColor = birincil ? MaviAcik : GriKoyu;
        b.FlatAppearance.MouseDownBackColor = birincil ? MaviKoyu : Color.FromArgb(180, 186, 194);
        b.EnabledChanged += delegate
        {
            b.BackColor = b.Enabled ? (birincil ? Mavi : Gri) : Color.FromArgb(214, 218, 223);
        };
        return b;
    }

    void OgeCiz(object sender, DrawItemEventArgs e)
    {
        if (e.Index < 0 || e.Index >= liste.Items.Count) return;
        var o = (Oyun)liste.Items[e.Index];
        var g = e.Graphics;
        g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
        bool secili = (e.State & DrawItemState.Selected) != 0;
        var r = e.Bounds;
        using (var b = new SolidBrush(secili ? Color.FromArgb(227, 239, 252) : Color.White)) g.FillRectangle(b, r);
        if (secili) using (var b = new SolidBrush(Mavi)) g.FillRectangle(b, r.Left, r.Top, (int)(4 * s), r.Height);
        using (var p = new Pen(Gri)) g.DrawLine(p, r.Left, r.Bottom - 1, r.Right, r.Bottom - 1);

        bool xbox = o.Tur.StartsWith("Xbox");
        string rozet = o.Tur.StartsWith("Steam") ? "STEAM" : (xbox ? "XBOX" : "KLASÖR");
        var rr = new Rectangle(r.Left + (int)(16 * s), r.Top + (r.Height - (int)(26 * s)) / 2, (int)(72 * s), (int)(26 * s));
        using (var b = new SolidBrush(xbox ? Mavi : Color.FromArgb(52, 58, 64))) g.FillRectangle(b, rr);
        using (var f = new Font("Segoe UI", 8.5f, FontStyle.Bold))
            TextRenderer.DrawText(g, rozet, f, rr, Color.White, TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter);

        int x = rr.Right + (int)(14 * s);
        int sag = r.Right - (int)(16 * s);
        if (o.Kurulu)
        {
            var kr = new Rectangle(sag - (int)(118 * s), r.Top + (r.Height - (int)(24 * s)) / 2, (int)(118 * s), (int)(24 * s));
            using (var b = new SolidBrush(Color.FromArgb(214, 234, 252))) g.FillRectangle(b, kr);
            using (var f = new Font("Segoe UI", 8.5f, FontStyle.Bold))
                TextRenderer.DrawText(g, "TÜRKÇE KURULU", f, kr, MaviKoyu, TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter);
            sag = kr.Left - (int)(10 * s);
        }
        using (var f = new Font("Segoe UI Semibold", 10.5f))
            TextRenderer.DrawText(g, "Forza Horizon 6  (" + o.Tur + ")", f, new Rectangle(x, r.Top + (int)(8 * s), sag - x, (int)(22 * s)), Yazi, TextFormatFlags.EndEllipsis | TextFormatFlags.NoPadding);
        using (var f = new Font("Segoe UI", 8.75f))
            TextRenderer.DrawText(g, o.Banka, f, new Rectangle(x, r.Top + (int)(32 * s), sag - x, (int)(20 * s)), YaziSoluk, TextFormatFlags.PathEllipsis | TextFormatFlags.NoPadding);
    }

    // ---------- Oyun bulma ----------
    void Tara()
    {
        liste.Items.Clear();
        var bulunan = new List<Oyun>();
        try { SteamAra(bulunan); } catch { }
        try { XboxAra(bulunan); } catch { }
        foreach (var o in bulunan)
            if (!liste.Items.Cast<Oyun>().Any(x => string.Equals(x.Banka, o.Banka, StringComparison.OrdinalIgnoreCase)))
                liste.Items.Add(o);
        if (liste.Items.Count > 0)
        {
            liste.SelectedIndex = 0;
            Yaz(liste.Items.Count + " kurulum bulundu. Listeden seçip \"Türkçe Dublajı Kur\" düğmesine basın.");
        }
        else
            Yaz("Oyun otomatik bulunamadı. \"Oyun klasörünü kendim seçeyim\" ile oyunun kurulu olduğu klasörü seçin.");
        if (!Directory.Exists(veri) || Directory.GetFiles(veri, "*.assets.bank").Length == 0)
            Yaz("UYARI: Kurulum.exe'nin yanında \"Veri\" klasörü bulunamadı. Zip'in tamamını bir klasöre çıkarıp öyle çalıştırın.");
    }

    static string BankaYolu(string oyunKlasoru)
    {
        string[] adaylar = {
            oyunKlasoru,
            Path.Combine(oyunKlasoru, @"media\Audio\FMODBanks"),
            Path.Combine(oyunKlasoru, @"Content\media\Audio\FMODBanks"),
            Path.Combine(oyunKlasoru, @"Audio\FMODBanks"),
            Path.Combine(oyunKlasoru, "FMODBanks")
        };
        foreach (var a in adaylar)
            if (File.Exists(Path.Combine(a, KONTROL)) || Directory.Exists(Path.Combine(a, YEDEK)))
                return a;
        return null;
    }

    void Ekle(List<Oyun> l, string tur, string klasor)
    {
        if (!Directory.Exists(klasor)) return;
        var b = BankaYolu(klasor);
        if (b != null) l.Add(new Oyun { Tur = tur, Banka = b });
    }

    void SteamAra(List<Oyun> l)
    {
        var kokler = new List<string>();
        foreach (var k in new[] {
            Registry.GetValue(@"HKEY_CURRENT_USER\Software\Valve\Steam", "SteamPath", null),
            Registry.GetValue(@"HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath", null),
            Registry.GetValue(@"HKEY_LOCAL_MACHINE\SOFTWARE\Valve\Steam", "InstallPath", null) })
            if (k != null) kokler.Add(k.ToString().Replace('/', '\\'));
        var kutuphaneler = new List<string>(kokler);
        foreach (var k in kokler)
        {
            var vdf = Path.Combine(k, @"steamapps\libraryfolders.vdf");
            if (!File.Exists(vdf)) continue;
            foreach (Match m in Regex.Matches(File.ReadAllText(vdf), "\"path\"\\s+\"([^\"]+)\""))
                kutuphaneler.Add(m.Groups[1].Value.Replace("\\\\", "\\"));
        }
        foreach (var d in DriveInfo.GetDrives().Where(x => x.DriveType == DriveType.Fixed && x.IsReady))
            foreach (var ad in new[] { "SteamLibrary", "Steam", "STEAM", @"Program Files (x86)\Steam", @"Games\Steam" })
                kutuphaneler.Add(Path.Combine(d.RootDirectory.FullName, ad));
        foreach (var lib in kutuphaneler.Distinct(StringComparer.OrdinalIgnoreCase))
        {
            var common = Path.Combine(lib, @"steamapps\common");
            if (!Directory.Exists(common)) continue;
            foreach (var dir in Directory.GetDirectories(common))
            {
                var ad = Path.GetFileName(dir).Replace(" ", "").ToLowerInvariant();
                if (ad.Contains("forzahorizon6")) Ekle(l, "Steam", dir);
            }
        }
    }

    void XboxAra(List<Oyun> l)
    {
        foreach (var d in DriveInfo.GetDrives().Where(x => x.DriveType == DriveType.Fixed && x.IsReady))
        {
            var kok = d.RootDirectory.FullName;
            var oyunKokleri = new List<string> { Path.Combine(kok, "XboxGames") };
            var gr = Path.Combine(kok, ".GamingRoot");
            if (File.Exists(gr))
            {
                try
                {
                    var b = File.ReadAllBytes(gr);
                    if (b.Length > 8)
                        foreach (var p in Encoding.Unicode.GetString(b, 8, b.Length - 8).Split('\0'))
                            if (p.Trim().Length > 0) oyunKokleri.Add(Path.Combine(kok, p.Trim()));
                }
                catch { }
            }
            foreach (var ok in oyunKokleri.Distinct(StringComparer.OrdinalIgnoreCase))
            {
                if (!Directory.Exists(ok)) continue;
                foreach (var dir in Directory.GetDirectories(ok))
                {
                    var ad = Path.GetFileName(dir).Replace(" ", "").ToLowerInvariant();
                    if (ad.Contains("forzahorizon6")) Ekle(l, "Xbox / Microsoft Store", dir);
                }
            }
        }
    }

    void ElleSec()
    {
        using (var f = new FolderBrowserDialog())
        {
            f.Description = "Forza Horizon 6'nın kurulu olduğu klasörü seçin\n(Steam: ...\\steamapps\\common\\ForzaHorizon6, Xbox: ...\\XboxGames\\Forza Horizon 6)";
            if (f.ShowDialog(this) != DialogResult.OK) return;
            var b = BankaYolu(f.SelectedPath);
            if (b == null)
            {
                MessageBox.Show(this, "Bu klasörde oyunun ses dosyaları (media\\Audio\\FMODBanks) bulunamadı.\nOyunun ana klasörünü seçtiğinizden emin olun.", "Bulunamadı", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                return;
            }
            var tur = b.ToLowerInvariant().Contains("steamapps") ? "Steam" : (b.ToLowerInvariant().Contains("xboxgames") ? "Xbox / Microsoft Store" : "Elle seçildi");
            var o = new Oyun { Tur = tur, Banka = b };
            liste.Items.Add(o);
            liste.SelectedItem = o;
            Yaz("Seçildi: " + b);
        }
    }

    // ---------- Kur / Kaldır ----------
    void Calistir(bool kur)
    {
        var o = liste.SelectedItem as Oyun;
        if (o == null) { MessageBox.Show(this, "Önce listeden bir oyun kurulumu seçin.", "Oyun seçilmedi"); return; }
        if (Process.GetProcesses().Any(p => p.ProcessName.ToLowerInvariant().Contains("forzahorizon6")))
        {
            MessageBox.Show(this, "Oyun açık görünüyor. Lütfen önce oyunu tamamen kapatın.", "Oyun açık", MessageBoxButtons.OK, MessageBoxIcon.Warning);
            return;
        }
        string[] dosyalar = new string[0];
        if (kur)
        {
            if (Directory.Exists(veri)) dosyalar = Directory.GetFiles(veri, "*.assets.bank");
            if (dosyalar.Length == 0) { MessageBox.Show(this, "\"Veri\" klasöründe ses dosyası yok. Zip'in tamamını çıkardığınızdan emin olun.", "Veri yok"); return; }
            long gerek = dosyalar.Sum(f => new FileInfo(f).Length);
            try
            {
                long bos = new DriveInfo(Path.GetPathRoot(o.Banka)).AvailableFreeSpace;
                if (bos < gerek + 200L * 1024 * 1024)
                {
                    MessageBox.Show(this, "Diskte yeterli boş yer yok.\nGereken: " + (gerek / 1073741824.0).ToString("0.0") + " GB, boş: " + (bos / 1073741824.0).ToString("0.0") + " GB", "Yer yok");
                    return;
                }
            }
            catch { }
        }
        else if (!o.Kurulu)
        {
            MessageBox.Show(this, "Bu kurulumda Türkçe dublaj yedeği bulunamadı; kaldırılacak bir şey yok.", "Kurulu değil");
            return;
        }
        Dugmeler(false);
        var t = new Thread(delegate ()
        {
            string sonuc;
            try { sonuc = kur ? Kur(o, dosyalar) : Kaldir(o); }
            catch (UnauthorizedAccessException ex) { sonuc = "HATA: Dosyalara yazma izni yok.\n" + YetkiIpucu(o) + "\n(" + ex.Message + ")"; }
            catch (IOException ex) { sonuc = "HATA: " + ex.Message + "\nOyun ve Steam/Xbox uygulaması dosyaları kullanıyor olabilir; kapatıp tekrar deneyin."; }
            catch (Exception ex) { sonuc = "HATA: " + ex.Message; }
            BeginInvoke((Action)delegate
            {
                Yaz(sonuc);
                Dugmeler(true);
                int i = liste.SelectedIndex; liste.Items[i] = liste.Items[i];
                MessageBox.Show(this, sonuc, "Forza Horizon 6 — Türkçe Dublaj", MessageBoxButtons.OK,
                    sonuc.StartsWith("HATA") ? MessageBoxIcon.Error : MessageBoxIcon.Information);
            });
        });
        t.IsBackground = true;
        t.Start();
    }

    static string YetkiIpucu(Oyun o)
    {
        if (o.Tur.StartsWith("Xbox"))
            return "Xbox sürümü için: Xbox uygulamasında oyuna sağ tıklayın > Yönet > Dosyalar > \"Gelişmiş yönetim özelliklerini etkinleştir\" seçeneğini açın, sonra tekrar deneyin.";
        return "Programı sağ tıklayıp \"Yönetici olarak çalıştır\" ile açmayı deneyin.";
    }

    string Kur(Oyun o, string[] dosyalar)
    {
        var yedek = Path.Combine(o.Banka, YEDEK);
        Directory.CreateDirectory(yedek);
        int n = 0, atlanan = 0;
        Ilerleme(0, dosyalar.Length);
        foreach (var src in dosyalar)
        {
            var ad = Path.GetFileName(src);
            var hedef = Path.Combine(o.Banka, ad);
            var bak = Path.Combine(yedek, ad);
            Yaz2("Kuruluyor: " + ad);
            if (!File.Exists(hedef) && !File.Exists(bak)) { atlanan++; Ilerleme(n + atlanan, dosyalar.Length); continue; }
            if (File.Exists(hedef))
            {
                bool bizim = new FileInfo(hedef).Length == new FileInfo(src).Length;
                if (!bizim)
                {
                    // oyundaki dosya orijinal (ilk kurulum ya da oyun güncellenmiş): yedeğe taşı
                    if (File.Exists(bak)) File.Delete(bak);
                    File.Move(hedef, bak);
                }
                else File.Delete(hedef);
            }
            File.Copy(src, hedef, true);
            n++;
            Ilerleme(n + atlanan, dosyalar.Length);
        }
        File.WriteAllText(Path.Combine(yedek, "kurulum_bilgisi.txt"),
            "FH6 Türkçe Dublaj kuruldu: " + DateTime.Now + "\r\nBu klasördeki dosyalar oyunun orijinal İngilizce ses dosyalarıdır. Silmeyin; kaldırma işlemi bunları geri koyar.\r\n", Encoding.UTF8);
        return "Türkçe dublaj kuruldu (" + n + " ses dosyası" + (atlanan > 0 ? ", " + atlanan + " dosya bu oyun sürümünde yok, atlandı" : "") + ").\n" +
               "Oyunu açıp keyfini çıkarın! Oyun güncellenirse ya da Steam'de \"dosya bütünlüğünü doğrula\" yapılırsa kurulumu tekrar çalıştırın.";
    }

    string Kaldir(Oyun o)
    {
        var yedek = Path.Combine(o.Banka, YEDEK);
        var bak = Directory.GetFiles(yedek, "*.assets.bank");
        int n = 0;
        Ilerleme(0, bak.Length);
        foreach (var b in bak)
        {
            var hedef = Path.Combine(o.Banka, Path.GetFileName(b));
            Yaz2("Geri yükleniyor: " + Path.GetFileName(b));
            if (File.Exists(hedef)) File.Delete(hedef);
            File.Move(b, hedef);
            n++;
            Ilerleme(n, bak.Length);
        }
        try { Directory.Delete(yedek, true); } catch { }
        return "Türkçe dublaj kaldırıldı, oyunun orijinal İngilizce sesleri geri yüklendi (" + n + " dosya).";
    }

    // ---------- Arayüz yardımcıları ----------
    void Dugmeler(bool acik) { btnKur.Enabled = btnKaldir.Enabled = btnSec.Enabled = btnTara.Enabled = liste.Enabled = acik; }
    void Yaz(string s2) { durum.Text = s2; }
    void Yaz2(string s2) { BeginInvoke((Action)delegate { durum.Text = s2; }); }
    void Ilerleme(int a, int b) { BeginInvoke((Action)delegate { pbMax = Math.Max(1, b); pbDeger = Math.Min(a, pbMax); pb.Invalidate(); }); }
}
