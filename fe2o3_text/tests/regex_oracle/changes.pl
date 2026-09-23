#!/usr/bin/env perl
# Writes props_changed.txt: every (property, code point) whose value the Unicode Character
# Database changed between Perl's Unicode version (props_expected.txt) and the version the tables
# are generated from, for the code points assigned in the older one.  tests/regex.rs accepts a
# disagreement with Perl only where it is listed here, so the list explains each difference by
# Unicode's own changes rather than by a tolerance.
#
# The files are fetched with curl and cached under the system temporary directory, as the table
# generator caches them.  Run from this directory:
#
#   perl changes.pl 15.0.0 17.0.0 > props_changed.txt
use strict;
use warnings;
use File::Spec;
use File::Path qw(make_path);

my ($old, $new) = @ARGV;
die "usage: changes.pl OLD NEW\n" unless $old && $new;

my @files = qw(
	ucd/UnicodeData.txt ucd/Scripts.txt ucd/ScriptExtensions.txt ucd/PropList.txt
	ucd/DerivedCoreProperties.txt ucd/emoji/emoji-data.txt
);

sub fetch {
	my ($ver, $path) = @_;
	my $dir = File::Spec->catdir(File::Spec->tmpdir, 'fe2o3_ucd', $ver);
	make_path($dir);
	(my $name = $path) =~ s{.*/}{};
	my $dest = File::Spec->catfile($dir, $name);
	unless (-s $dest) {
		system('curl', '-sS', '--fail', '-o', $dest,
			"https://www.unicode.org/Public/$ver/$path") == 0 or die "curl failed for $path\n";
	}
	open my $fh, '<', $dest or die "$dest: $!\n";
	local $/;
	return <$fh>;
}

# Calls $cb->(lo, hi, @fields) for each data line of a property file.
sub each_range {
	my ($text, $cb) = @_;
	for my $line (split /\n/, $text) {
		$line =~ s/#.*//;
		next unless $line =~ /\S/;
		my @f = map { s/^\s+|\s+$//gr } split /;/, $line;
		my ($lo, $hi) = $f[0] =~ /^([0-9A-F]+)(?:\.\.([0-9A-F]+))?$/ or die "bad range $f[0]\n";
		$cb->(hex $lo, hex($hi // $lo), @f[1 .. $#f]);
	}
}

# Loads one version: gc, sc, raw scx and the binary properties, keyed by code point.
sub load {
	my $ver = shift;
	my %d = (gc => {}, sc => {}, scx => {}, bin => {});
	my $first;
	for my $line (split /\n/, fetch($ver, 'ucd/UnicodeData.txt')) {
		my @f = split /;/, $line;
		my $cp = hex $f[0];
		if ($f[1] =~ /, First>$/) { $first = $cp; next; }
		my $lo = $f[1] =~ /, Last>$/ ? $first : $cp;
		$d{gc}{$_} = $f[2] for $lo .. $cp;
	}
	each_range(fetch($ver, 'ucd/Scripts.txt'), sub { my ($lo, $hi, $v) = @_; $d{sc}{$_} = $v for $lo .. $hi });
	each_range(fetch($ver, 'ucd/ScriptExtensions.txt'), sub {
		my ($lo, $hi, $v) = @_;
		$d{scx}{$_} = join ' ', sort split ' ', $v for $lo .. $hi;
	});
	for my $f ('ucd/PropList.txt', 'ucd/DerivedCoreProperties.txt', 'ucd/emoji/emoji-data.txt') {
		each_range(fetch($ver, $f), sub {
			my ($lo, $hi, @v) = @_;
			return unless @v == 1;
			$d{bin}{$v[0]}{$_} = 1 for $lo .. $hi;
		});
	}
	return \%d;
}

my ($da, $db) = (load($old), load($new));
my %props = map { $_ => 1 } (keys %{ $da->{bin} }, keys %{ $db->{bin} });
for my $cp (sort { $a <=> $b } keys %{ $da->{gc} }) {
	my $g = sub { $_[0] // '' };
	printf "gc\t%X\n", $cp if $g->($da->{gc}{$cp}) ne $g->($db->{gc}{$cp});
	my ($sa, $sb) = ($da->{sc}{$cp} // 'Unknown', $db->{sc}{$cp} // 'Unknown');
	printf "sc\t%X\n", $cp if $sa ne $sb;
	my ($xa, $xb) = ($da->{scx}{$cp} // "=$sa", $db->{scx}{$cp} // "=$sb");
	printf "scx\t%X\n", $cp if $xa ne $xb;
	for my $p (sort keys %props) {
		my $in_a = $da->{bin}{$p} && $da->{bin}{$p}{$cp} ? 1 : 0;
		my $in_b = $db->{bin}{$p} && $db->{bin}{$p}{$cp} ? 1 : 0;
		printf "%s\t%X\n", $p, $cp if $in_a != $in_b;
	}
}
