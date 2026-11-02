package dev.horizonlab.parity;
import terrablender.worldgen.noise.*;
/** Runs the original TerraBlender layer implementations, not a translated test stub. */
public final class RegionWeightOracle {
    static Area area(long seed, int zooms, int[] weights) {
        int total = java.util.Arrays.stream(weights).sum();
        AreaTransformer0 initial = (context,x,z) -> {
            int roll=context.nextRandom(total);
            for(int i=0;i<weights.length;i++){roll-=weights[i];if(roll<0)return i;}
            throw new AssertionError();
        };
        AreaFactory factory=initial.run(new AreaContext(25,seed,1));
        factory=ZoomLayer.FUZZY.run(new AreaContext(25,seed,2000),factory);
        for(int i=0;i<3;i++)factory=ZoomLayer.NORMAL.run(new AreaContext(25,seed,2001+i),factory);
        for(int i=0;i<zooms;i++)factory=ZoomLayer.NORMAL.run(new AreaContext(25,seed,1001+i),factory);
        return factory.make();
    }
    public static void main(String[] args) {
        long[] seeds={0,1,-1,8448363262698333288L,Long.MIN_VALUE,Long.MAX_VALUE};
        int[][] settings={{10,10,8,2},{10,13,2},{10,0,0,0},{10,1,0,50},{1,Integer.MAX_VALUE-1,0,0}};
        for(long seed:seeds)for(int mode=0;mode<settings.length;mode++){
            int zooms=mode==1?2:3; Area a=area(seed,zooms,settings[mode]);
            for(int i=0;i<256;i++){
                int x=i<32?i-16:(int)(i*104729L%15000001)-7500000;
                int z=i<32?15-i:(int)(i*17389L%15000001)-7500000;
                System.out.println(seed+","+mode+","+x+","+z+","+a.get(x,z));
            }
        }
    }
}
